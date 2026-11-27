//! Dedicated local loopback binary TCP data plane for live Arrow IPC exchange
//! between Rust (`scires-interchange`) and a running Julia kernel process.
//!
//! Arrow IPC streams are transferred over a length-prefixed binary TCP frame on
//! `127.0.0.1` authenticated with a per-session random token, never embedded in
//! Jupyter JSON control messages.

use scires_kernel::KernelError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Julia module loaded silently into `Main.__SciResDataPlane` during handshake.
pub(crate) const JULIA_DATAPLANE_SOURCE: &str = include_str!("../julia/scires_dataplane.jl");

const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct FrameRequest<'a> {
    token: &'a str,
    op: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct FrameResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Client for the live Julia loopback binary Arrow data plane.
#[derive(Debug, Clone)]
pub(crate) struct DataPlaneClient {
    addr: SocketAddr,
    token: String,
    timeout: Duration,
}

impl DataPlaneClient {
    /// Construct a client targeting `127.0.0.1:port` with `token`.
    pub(crate) fn new(port: u16, token: String, timeout: Duration) -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            token,
            timeout,
        }
    }

    /// Verify connectivity and token authentication against the Julia data-plane server.
    pub(crate) fn ping(&self) -> Result<(), KernelError> {
        let _ = self.round_trip("ping", None, &[])?;
        Ok(())
    }

    /// Export a variable `name` from `Main` as raw `scires.value` v1 Arrow IPC bytes,
    /// validating the received stream via `scires_interchange::decode`.
    pub(crate) fn export_value(&self, name: &str) -> Result<Vec<u8>, KernelError> {
        validate_identifier(name)?;
        let bytes = self.round_trip("export", Some(name), &[])?;
        scires_interchange::decode(&bytes).map_err(|error| {
            KernelError::Transfer(format!(
                "Julia exported invalid scires.value v1 payload for `{name}`: {error}"
            ))
        })?;
        Ok(bytes)
    }

    /// Validate `payload` via `scires_interchange::decode` and import it into `Main.<name>`.
    pub(crate) fn import_value(&self, name: &str, payload: &[u8]) -> Result<(), KernelError> {
        validate_identifier(name)?;
        scires_interchange::decode(payload).map_err(|error| {
            KernelError::Transfer(format!(
                "refusing to import invalid scires.value v1 payload for `{name}`: {error}"
            ))
        })?;
        let _ = self.round_trip("import", Some(name), payload)?;
        Ok(())
    }

    /// Best-effort shutdown notification to close the Julia loopback listener.
    pub(crate) fn shutdown(&self) {
        let _ = self.round_trip("shutdown", None, &[]);
    }

    fn round_trip(
        &self,
        op: &str,
        name: Option<&str>,
        payload: &[u8],
    ) -> Result<Vec<u8>, KernelError> {
        let mut stream = TcpStream::connect_timeout(&self.addr, self.timeout).map_err(|error| {
            KernelError::Transport(format!(
                "failed to connect to Julia Arrow data plane at {}: {error}",
                self.addr
            ))
        })?;
        let _ = stream.set_read_timeout(Some(self.timeout));
        let _ = stream.set_write_timeout(Some(self.timeout));

        let req = FrameRequest {
            token: &self.token,
            op,
            name,
        };
        let header = serde_json::to_vec(&req).map_err(|error| {
            KernelError::Protocol(format!("failed to serialize data-plane header: {error}"))
        })?;
        write_frame(&mut stream, &header, payload)?;

        let (resp_header, resp_payload) = read_frame(&mut stream)?;
        let resp: FrameResponse = serde_json::from_slice(&resp_header).map_err(|error| {
            KernelError::Protocol(format!("invalid data-plane response header: {error}"))
        })?;
        if !resp.ok {
            return Err(KernelError::Transfer(
                resp.error
                    .unwrap_or_else(|| format!("data-plane `{op}` failed")),
            ));
        }
        Ok(resp_payload)
    }
}

/// Validate that `name` is a simple Julia variable identifier.
pub(crate) fn validate_identifier(name: &str) -> Result<(), KernelError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(KernelError::Transfer(
            "variable name must not be empty".into(),
        ));
    };
    if !(first.is_alphabetic() || first == '_') {
        return Err(KernelError::Transfer(format!(
            "invalid Julia variable identifier `{name}`"
        )));
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '!') {
        return Err(KernelError::Transfer(format!(
            "invalid Julia variable identifier `{name}`"
        )));
    }
    Ok(())
}

/// Write one length-prefixed binary data-plane frame (`u32` header len, header, `u64` payload len, payload).
pub(crate) fn write_frame<W: Write>(
    writer: &mut W,
    header: &[u8],
    payload: &[u8],
) -> Result<(), KernelError> {
    let header_len = u32::try_from(header.len()).map_err(|_| {
        KernelError::Protocol("data-plane header length exceeds u32".into())
    })?;
    if header.len() > MAX_HEADER_BYTES {
        return Err(KernelError::Protocol(
            "data-plane header exceeds maximum allowed size".into(),
        ));
    }
    let payload_len = u64::try_from(payload.len()).map_err(|_| {
        KernelError::Protocol("data-plane payload length exceeds u64".into())
    })?;
    writer
        .write_all(&header_len.to_be_bytes())
        .and_then(|_| writer.write_all(header))
        .and_then(|_| writer.write_all(&payload_len.to_be_bytes()))
        .and_then(|_| writer.write_all(payload))
        .and_then(|_| writer.flush())
        .map_err(|error| {
            KernelError::Transport(format!("failed to write data-plane frame: {error}"))
        })
}

/// Read one length-prefixed binary data-plane frame.
pub(crate) fn read_frame<R: Read>(reader: &mut R) -> Result<(Vec<u8>, Vec<u8>), KernelError> {
    let mut hdr_len_buf = [0_u8; 4];
    reader.read_exact(&mut hdr_len_buf).map_err(|error| {
        KernelError::Transport(format!("failed to read data-plane header length: {error}"))
    })?;
    let hdr_len = u32::from_be_bytes(hdr_len_buf) as usize;
    if hdr_len > MAX_HEADER_BYTES {
        return Err(KernelError::Protocol(format!(
            "data-plane header length {hdr_len} exceeds limit {MAX_HEADER_BYTES}"
        )));
    }
    let mut header = vec![0_u8; hdr_len];
    reader.read_exact(&mut header).map_err(|error| {
        KernelError::Transport(format!("failed to read data-plane header: {error}"))
    })?;

    let mut payload_len_buf = [0_u8; 8];
    reader.read_exact(&mut payload_len_buf).map_err(|error| {
        KernelError::Transport(format!("failed to read data-plane payload length: {error}"))
    })?;
    let payload_len = usize::try_from(u64::from_be_bytes(payload_len_buf)).map_err(|_| {
        KernelError::Protocol("data-plane payload length exceeds host usize".into())
    })?;
    let mut payload = vec![0_u8; payload_len];
    reader.read_exact(&mut payload).map_err(|error| {
        KernelError::Transport(format!("failed to read data-plane payload: {error}"))
    })?;
    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scires_interchange::{encode, InterchangeValue, ScalarValue};
    use std::io::Cursor;
    use std::net::TcpListener;

    #[test]
    fn binary_frame_round_trip_and_oversized_header_rejection() {
        let mut buf = Vec::new();
        write_frame(&mut buf, br#"{"ok":true}"#, b"arrow-bytes").expect("frame writes");
        let (hdr, payload) = read_frame(&mut Cursor::new(&buf)).expect("frame reads");
        assert_eq!(hdr, br#"{"ok":true}"#);
        assert_eq!(payload, b"arrow-bytes");

        let mut bad = Vec::new();
        bad.extend_from_slice(&((MAX_HEADER_BYTES as u32) + 1).to_be_bytes());
        assert!(matches!(
            read_frame(&mut Cursor::new(&bad)),
            Err(KernelError::Protocol(_))
        ));
    }

    #[test]
    fn identifier_validation_rejects_injection_and_empty() {
        assert!(validate_identifier("x").is_ok());
        assert!(validate_identifier("matrix_1").is_ok());
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("1abc").is_err());
        assert!(validate_identifier("x; rm -rf /").is_err());
    }

    #[test]
    fn client_enforces_token_auth_and_validates_arrow_payload() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let expected_token = "secret-token".to_owned();
        let server_token = expected_token.clone();

        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().unwrap();
                let (hdr, _payload) = read_frame(&mut sock).unwrap();
                let req: serde_json::Value = serde_json::from_slice(&hdr).unwrap();
                if req["token"] != server_token {
                    write_frame(
                        &mut sock,
                        br#"{"ok":false,"error":"invalid data-plane token"}"#,
                        &[],
                    )
                    .unwrap();
                } else {
                    write_frame(&mut sock, br#"{"ok":true}"#, &[]).unwrap();
                }
            }
        });

        let bad_client = DataPlaneClient::new(port, "wrong-token".into(), Duration::from_secs(2));
        assert!(matches!(
            bad_client.ping(),
            Err(KernelError::Transfer(msg)) if msg.contains("invalid data-plane token")
        ));

        let good_client = DataPlaneClient::new(port, expected_token, Duration::from_secs(2));
        // Invalid Arrow bytes must be rejected locally before touching the server.
        assert!(matches!(
            good_client.import_value("x", b"not-arrow"),
            Err(KernelError::Transfer(_))
        ));

        // Valid Arrow IPC payload succeeds.
        let valid_bytes =
            encode(&InterchangeValue::Scalar(ScalarValue::Int32(Some(7)))).unwrap();
        good_client.import_value("x", &valid_bytes).unwrap();
        server.join().unwrap();
    }
}
