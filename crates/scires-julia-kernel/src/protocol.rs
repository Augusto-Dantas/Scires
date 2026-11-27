//! Pure Jupyter Wire Protocol v5.3 framing, HMAC-SHA256 signing, connection
//! metadata, and IOPub/shell correlation state machine.

use hmac::{Hmac, Mac};
use scires_kernel::{ExecuteResult, KernelError, Output, RequestId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Jupyter wire protocol delimiter separating ZeroMQ identities from the signed payload.
pub(crate) const DELIMITER: &[u8] = b"<IDS|MSG>";

/// Jupyter messaging protocol version emitted in outbound headers.
pub(crate) const PROTOCOL_VERSION: &str = "5.3";

/// Jupyter kernel connection parameters written to the temporary connection file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConnectionInfo {
    pub shell_port: u16,
    pub iopub_port: u16,
    pub stdin_port: u16,
    pub control_port: u16,
    pub hb_port: u16,
    pub ip: String,
    pub key: String,
    pub transport: String,
    pub signature_scheme: String,
    pub kernel_name: String,
}

impl ConnectionInfo {
    /// Reserve five distinct ephemeral loopback TCP ports and generate a random HMAC key.
    pub(crate) fn allocate_ephemeral() -> Result<Self, KernelError> {
        let listeners: Vec<TcpListener> = (0..5)
            .map(|_| {
                TcpListener::bind("127.0.0.1:0").map_err(|error| {
                    KernelError::Startup(format!("failed to allocate loopback TCP port: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let ports: Vec<u16> = listeners
            .iter()
            .map(|listener| {
                listener.local_addr().map(|addr| addr.port()).map_err(|error| {
                    KernelError::Startup(format!("failed to inspect loopback TCP port: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Release all five listeners together once distinct ports have been assigned.
        drop(listeners);

        let key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        Ok(Self {
            shell_port: ports[0],
            iopub_port: ports[1],
            stdin_port: ports[2],
            control_port: ports[3],
            hb_port: ports[4],
            ip: "127.0.0.1".into(),
            key,
            transport: "tcp".into(),
            signature_scheme: "hmac-sha256".into(),
            kernel_name: "scires-julia".into(),
        })
    }

    /// Format a ZeroMQ TCP endpoint URI for one of the connection ports.
    pub(crate) fn endpoint(&self, port: u16) -> String {
        format!("{}://{}:{}", self.transport, self.ip, port)
    }
}

/// RAII wrapper for a temporary Jupyter connection JSON file.
#[derive(Debug)]
pub(crate) struct TempConnectionFile {
    path: PathBuf,
}

impl TempConnectionFile {
    /// Write `info` as JSON into a fresh file in the system temporary directory.
    pub(crate) fn create(info: &ConnectionInfo) -> Result<Self, KernelError> {
        let path = std::env::temp_dir().join(format!("scires-julia-conn-{}.json", Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(info).map_err(|error| {
            KernelError::Startup(format!("failed to serialize connection info: {error}"))
        })?;
        fs::write(&path, bytes).map_err(|error| {
            KernelError::Startup(format!(
                "failed to write connection file {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }

    /// Path to the temporary connection file.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempConnectionFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Jupyter message header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JupyterHeader {
    pub msg_id: String,
    pub session: String,
    pub username: String,
    #[serde(default)]
    pub date: String,
    pub msg_type: String,
    pub version: String,
}

/// Decoded Jupyter wire message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JupyterMessage {
    pub identities: Vec<Vec<u8>>,
    pub header: JupyterHeader,
    pub parent_header: Value,
    pub metadata: Value,
    pub content: Value,
    pub buffers: Vec<Vec<u8>>,
}

impl JupyterMessage {
    /// Construct a new outbound Jupyter request message.
    pub(crate) fn new_request(
        session: &str,
        msg_id: impl Into<String>,
        msg_type: &str,
        content: Value,
    ) -> Self {
        Self {
            identities: Vec::new(),
            header: JupyterHeader {
                msg_id: msg_id.into(),
                session: session.to_owned(),
                username: "scires".into(),
                date: String::new(),
                msg_type: msg_type.to_owned(),
                version: PROTOCOL_VERSION.to_owned(),
            },
            parent_header: json!({}),
            metadata: json!({}),
            content,
            buffers: Vec::new(),
        }
    }

    /// Return `parent_header.msg_id` if present.
    pub(crate) fn parent_msg_id(&self) -> Option<&str> {
        self.parent_header.get("msg_id").and_then(Value::as_str)
    }

    /// Encode this message into Jupyter ZMTP multipart frames signed with `key`.
    pub(crate) fn encode(&self, key: &str) -> Result<Vec<Vec<u8>>, KernelError> {
        let header = serde_json::to_vec(&self.header).map_err(|error| {
            KernelError::Protocol(format!("failed to serialize Jupyter header: {error}"))
        })?;
        let parent = serde_json::to_vec(&self.parent_header).map_err(|error| {
            KernelError::Protocol(format!("failed to serialize parent_header: {error}"))
        })?;
        let metadata = serde_json::to_vec(&self.metadata).map_err(|error| {
            KernelError::Protocol(format!("failed to serialize metadata: {error}"))
        })?;
        let content = serde_json::to_vec(&self.content).map_err(|error| {
            KernelError::Protocol(format!("failed to serialize content: {error}"))
        })?;

        let signature = compute_hmac(key, &header, &parent, &metadata, &content)?;

        let mut frames = Vec::with_capacity(self.identities.len() + 6 + self.buffers.len());
        frames.extend(self.identities.iter().cloned());
        frames.push(DELIMITER.to_vec());
        frames.push(signature.into_bytes());
        frames.push(header);
        frames.push(parent);
        frames.push(metadata);
        frames.push(content);
        frames.extend(self.buffers.iter().cloned());
        Ok(frames)
    }

    /// Decode a Jupyter ZMTP multipart frame sequence and verify its HMAC-SHA256 signature.
    pub(crate) fn decode(frames: &[&[u8]], key: &str) -> Result<Self, KernelError> {
        let delim_pos = frames
            .iter()
            .position(|frame| *frame == DELIMITER)
            .ok_or_else(|| KernelError::Protocol("Jupyter message missing <IDS|MSG> delimiter".into()))?;

        if frames.len() < delim_pos + 6 {
            return Err(KernelError::Protocol(format!(
                "truncated Jupyter message: expected at least {} frames after delimiter, got {}",
                5,
                frames.len() - delim_pos - 1
            )));
        }

        let identities = frames[..delim_pos]
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        let sig_frame = frames[delim_pos + 1];
        let header_frame = frames[delim_pos + 2];
        let parent_frame = frames[delim_pos + 3];
        let metadata_frame = frames[delim_pos + 4];
        let content_frame = frames[delim_pos + 5];
        let buffers = frames[delim_pos + 6..]
            .iter()
            .map(|frame| frame.to_vec())
            .collect();

        verify_hmac(
            key,
            sig_frame,
            header_frame,
            parent_frame,
            metadata_frame,
            content_frame,
        )?;

        let header: JupyterHeader = serde_json::from_slice(header_frame).map_err(|error| {
            KernelError::Protocol(format!("invalid Jupyter header JSON: {error}"))
        })?;
        let parent_header: Value = serde_json::from_slice(parent_frame).map_err(|error| {
            KernelError::Protocol(format!("invalid Jupyter parent_header JSON: {error}"))
        })?;
        let metadata: Value = serde_json::from_slice(metadata_frame).map_err(|error| {
            KernelError::Protocol(format!("invalid Jupyter metadata JSON: {error}"))
        })?;
        let content: Value = serde_json::from_slice(content_frame).map_err(|error| {
            KernelError::Protocol(format!("invalid Jupyter content JSON: {error}"))
        })?;

        Ok(Self {
            identities,
            header,
            parent_header,
            metadata,
            content,
            buffers,
        })
    }
}

fn compute_hmac(
    key: &str,
    header: &[u8],
    parent: &[u8],
    metadata: &[u8],
    content: &[u8],
) -> Result<String, KernelError> {
    if key.is_empty() {
        return Ok(String::new());
    }
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|error| KernelError::Protocol(format!("invalid HMAC key: {error}")))?;
    mac.update(header);
    mac.update(parent);
    mac.update(metadata);
    mac.update(content);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn verify_hmac(
    key: &str,
    signature_hex: &[u8],
    header: &[u8],
    parent: &[u8],
    metadata: &[u8],
    content: &[u8],
) -> Result<(), KernelError> {
    if key.is_empty() {
        return Ok(());
    }
    let sig_bytes = hex::decode(signature_hex)
        .map_err(|_| KernelError::Protocol("malformed hex Jupyter HMAC signature".into()))?;
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|error| KernelError::Protocol(format!("invalid HMAC key: {error}")))?;
    mac.update(header);
    mac.update(parent);
    mac.update(metadata);
    mac.update(content);
    mac.verify_slice(&sig_bytes)
        .map_err(|_| KernelError::Protocol("invalid Jupyter HMAC signature".into()))
}

/// Correlates IOPub messages and the shell `execute_reply` for one request.
#[derive(Debug)]
pub(crate) struct ExecutionAccumulator {
    request_id: RequestId,
    outputs: Vec<Output>,
    iopub_error: Option<(String, String, Vec<String>)>,
    saw_idle: bool,
}

impl ExecutionAccumulator {
    /// Start accumulating outputs for `request_id`.
    pub(crate) fn new(request_id: RequestId) -> Self {
        Self {
            request_id,
            outputs: Vec::new(),
            iopub_error: None,
            saw_idle: false,
        }
    }

    /// Returns `true` once `status: idle` has been observed for this request.
    pub(crate) fn is_idle(&self) -> bool {
        self.saw_idle
    }

    /// Process one IOPub message; ignores messages belonging to other requests.
    pub(crate) fn handle_iopub(&mut self, msg: &JupyterMessage) {
        if msg.parent_msg_id() != Some(self.request_id.as_str()) {
            return;
        }
        match msg.header.msg_type.as_str() {
            "status" => {
                if msg.content.get("execution_state").and_then(Value::as_str) == Some("idle") {
                    self.saw_idle = true;
                }
            }
            "stream" => {
                let text = msg
                    .content
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let is_stderr = msg.content.get("name").and_then(Value::as_str) == Some("stderr");
                if is_stderr {
                    self.outputs.push(Output::Stderr(text));
                } else {
                    self.outputs.push(Output::Stdout(text));
                }
            }
            "display_data" | "execute_result" => {
                if let Some(bundle) = msg.content.get("data").and_then(Value::as_object) {
                    let mut entries: Vec<(&String, &Value)> = bundle.iter().collect();
                    entries.sort_by_key(|(mime, _)| *mime);
                    for (mime, value) in entries {
                        let data = match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        };
                        self.outputs.push(Output::Display {
                            mime: mime.clone(),
                            data,
                        });
                    }
                }
            }
            "error" => {
                let ename = msg
                    .content
                    .get("ename")
                    .and_then(Value::as_str)
                    .unwrap_or("ExecutionError")
                    .to_owned();
                let evalue = msg
                    .content
                    .get("evalue")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let traceback = extract_traceback(&msg.content);
                self.iopub_error = Some((ename, evalue, traceback));
            }
            _ => {}
        }
    }

    /// Finalize execution once the matching shell `execute_reply` has arrived.
    pub(crate) fn finish(self, reply: &JupyterMessage) -> Result<ExecuteResult, KernelError> {
        if reply.parent_msg_id() != Some(self.request_id.as_str()) {
            return Err(KernelError::Protocol(format!(
                "shell reply parent_header.msg_id {:?} does not match request {}",
                reply.parent_msg_id(),
                self.request_id
            )));
        }

        let status = reply
            .content
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("error");

        match status {
            "ok" => {
                let execution_count = reply
                    .content
                    .get("execution_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                Ok(ExecuteResult {
                    request_id: self.request_id,
                    execution_count,
                    outputs: self.outputs,
                })
            }
            "abort" => Err(KernelError::Interrupted),
            "error" => {
                let (fallback_ename, fallback_evalue, fallback_tb) = self
                    .iopub_error
                    .unwrap_or_else(|| ("ExecutionError".into(), "execution failed".into(), vec![]));
                let ename = reply
                    .content
                    .get("ename")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or(fallback_ename);
                let evalue = reply
                    .content
                    .get("evalue")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or(fallback_evalue);
                let traceback = {
                    let reply_tb = extract_traceback(&reply.content);
                    if reply_tb.is_empty() {
                        fallback_tb
                    } else {
                        reply_tb
                    }
                };
                if ename == "InterruptException" || evalue.starts_with("InterruptException") {
                    return Err(KernelError::Interrupted);
                }
                Err(KernelError::Execution {
                    ename,
                    evalue,
                    traceback,
                })
            }
            other => Err(KernelError::Protocol(format!(
                "unexpected execute_reply status `{other}`"
            ))),
        }
    }
}

fn extract_traceback(content: &Value) -> Vec<String> {
    content
        .get("traceback")
        .and_then(Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_message_round_trip_and_hmac_verification() {
        let key = "0123456789abcdef0123456789abcdef";
        let mut msg = JupyterMessage::new_request(
            "sess-1",
            "req-42",
            "execute_request",
            json!({"code": "1 + 1", "silent": false}),
        );
        msg.identities = vec![b"topic".to_vec()];
        msg.buffers = vec![vec![1, 2, 3, 4]];

        let encoded = msg.encode(key).expect("message should encode");
        let refs: Vec<&[u8]> = encoded.iter().map(Vec::as_slice).collect();
        let decoded = JupyterMessage::decode(&refs, key).expect("message should decode");
        assert_eq!(decoded, msg);

        // Wrong key must be rejected.
        let bad = JupyterMessage::decode(&refs, "wrong-key");
        assert!(matches!(bad, Err(KernelError::Protocol(_))));

        // Tampered content must be rejected.
        let mut tampered = encoded;
        let content_idx = tampered.len() - 2;
        tampered[content_idx] = br#"{"code":"tampered"}"#.to_vec();
        let tampered_refs: Vec<&[u8]> = tampered.iter().map(Vec::as_slice).collect();
        assert!(matches!(
            JupyterMessage::decode(&tampered_refs, key),
            Err(KernelError::Protocol(_))
        ));
    }

    #[test]
    fn connection_file_is_created_and_cleaned_up_on_drop() {
        let info = ConnectionInfo::allocate_ephemeral().expect("ports should allocate");
        let unique_ports: std::collections::HashSet<u16> = [
            info.shell_port,
            info.iopub_port,
            info.stdin_port,
            info.control_port,
            info.hb_port,
        ]
        .into_iter()
        .collect();
        assert_eq!(unique_ports.len(), 5);
        assert_eq!(info.signature_scheme, "hmac-sha256");

        let path = {
            let file = TempConnectionFile::create(&info).expect("file should be written");
            let path = file.path().to_path_buf();
            assert!(path.is_file());
            let parsed: ConnectionInfo =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(parsed, info);
            path
        };
        assert!(!path.exists(), "temp connection file must be removed on drop");
    }

    #[test]
    fn accumulator_correlates_outputs_preserves_mimes_and_filters_other_requests() {
        let req_id = RequestId::new("req-target");
        let mut acc = ExecutionAccumulator::new(req_id.clone());

        let mut unrelated = JupyterMessage::new_request(
            "s",
            "m0",
            "stream",
            json!({"name": "stdout", "text": "noise"}),
        );
        unrelated.parent_header = json!({"msg_id": "req-other"});
        acc.handle_iopub(&unrelated);

        let mut stdout = JupyterMessage::new_request(
            "s",
            "m1",
            "stream",
            json!({"name": "stdout", "text": "hello\n"}),
        );
        stdout.parent_header = json!({"msg_id": "req-target"});
        acc.handle_iopub(&stdout);

        let mut display = JupyterMessage::new_request(
            "s",
            "m2",
            "display_data",
            json!({
                "data": {
                    "text/plain": "Plot(...)",
                    "application/vnd.scires.plot+json": {"kind": "line"}
                }
            }),
        );
        display.parent_header = json!({"msg_id": "req-target"});
        acc.handle_iopub(&display);

        let mut idle = JupyterMessage::new_request(
            "s",
            "m3",
            "status",
            json!({"execution_state": "idle"}),
        );
        idle.parent_header = json!({"msg_id": "req-target"});
        acc.handle_iopub(&idle);
        assert!(acc.is_idle());

        let mut reply = JupyterMessage::new_request(
            "s",
            "m4",
            "execute_reply",
            json!({"status": "ok", "execution_count": 3}),
        );
        reply.parent_header = json!({"msg_id": "req-target"});

        let result = acc.finish(&reply).expect("execution should succeed");
        assert_eq!(result.request_id, req_id);
        assert_eq!(result.execution_count, 3);
        assert_eq!(
            result.outputs,
            vec![
                Output::Stdout("hello\n".into()),
                Output::Display {
                    mime: "application/vnd.scires.plot+json".into(),
                    data: "{\"kind\":\"line\"}".into(),
                },
                Output::Display {
                    mime: "text/plain".into(),
                    data: "Plot(...)".into(),
                },
            ]
        );
    }

    #[test]
    fn accumulator_maps_execution_error_and_interrupt_exception() {
        let req_id = RequestId::new("req-err");
        let acc = ExecutionAccumulator::new(req_id);
        let mut reply = JupyterMessage::new_request(
            "s",
            "m1",
            "execute_reply",
            json!({
                "status": "error",
                "ename": "DomainError",
                "evalue": "sqrt(-1)",
                "traceback": ["DomainError with -1.0:", "Stacktrace:"]
            }),
        );
        reply.parent_header = json!({"msg_id": "req-err"});
        let err = acc.finish(&reply).unwrap_err();
        assert_eq!(
            err,
            KernelError::Execution {
                ename: "DomainError".into(),
                evalue: "sqrt(-1)".into(),
                traceback: vec!["DomainError with -1.0:".into(), "Stacktrace:".into()],
            }
        );

        let acc_int = ExecutionAccumulator::new(RequestId::new("req-int"));
        let mut int_reply = JupyterMessage::new_request(
            "s",
            "m2",
            "execute_reply",
            json!({
                "status": "error",
                "ename": "InterruptException",
                "evalue": "",
                "traceback": []
            }),
        );
        int_reply.parent_header = json!({"msg_id": "req-int"});
        assert_eq!(acc_int.finish(&int_reply), Err(KernelError::Interrupted));

        // IJulia wraps top-level InterruptException in LoadError with evalue "InterruptException:".
        let acc_wrapped = ExecutionAccumulator::new(RequestId::new("req-int-2"));
        let mut wrapped_reply = JupyterMessage::new_request(
            "s",
            "m3",
            "execute_reply",
            json!({
                "status": "error",
                "ename": "LoadError",
                "evalue": "InterruptException:",
                "traceback": ["InterruptException:"]
            }),
        );
        wrapped_reply.parent_header = json!({"msg_id": "req-int-2"});
        assert_eq!(
            acc_wrapped.finish(&wrapped_reply),
            Err(KernelError::Interrupted)
        );
    }
}
