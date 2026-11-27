//! Kernel abstraction (port) and kernel manager.
//!
//! Concrete kernels (Jupyter/ZeroMQ adapters for Julia, Python, R, C/C++) live in
//! language-adapter crates or plugins and implement [`Kernel`] + [`KernelFactory`].
//! [`KernelManager`] owns registered factories, validates versioned handshakes,
//! negotiates supported value profiles, and lazily starts kernels on first use.
//!
//! # Examples
//!
//! ```
//! use std::sync::Arc;
//! use scires_core::{InMemoryEventBus, Language};
//! use scires_kernel::{mock::EchoKernelFactory, ExecuteRequest, KernelManager, Output};
//!
//! let mut mgr = KernelManager::new(Arc::new(InMemoryEventBus::new()));
//! mgr.register(Arc::new(EchoKernelFactory::new(Language::Julia)));
//!
//! let req = ExecuteRequest::new("1 + 1");
//! let res = mgr.execute(&Language::Julia, &req)?;
//! assert_eq!(res.request_id, req.request_id);
//! assert_eq!(res.outputs, vec![Output::Stdout("1 + 1".into())]);
//! # Ok::<(), scires_kernel::KernelError>(())
//! ```

mod manager;
pub mod mock;

pub use manager::KernelManager;

use scires_core::{Language, Variable};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use thiserror::Error;

/// SciRes kernel API identifier advertised in [`KernelHandshake`].
pub const KERNEL_API_ID: &str = "scires.kernel";

/// Supported SciRes kernel API major version.
pub const KERNEL_API_VERSION: u16 = 1;

/// SciRes Arrow value profile identifier.
pub const VALUE_PROFILE_ID: &str = "scires.value";

/// Supported SciRes Arrow value profile version.
pub const VALUE_PROFILE_VERSION: u16 = 1;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Unique host-assigned identifier used to correlate execution requests,
/// streaming outputs, and replies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RequestId(String);

impl RequestId {
    /// Create a request identifier from an explicit string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate a fresh, process-unique request identifier.
    pub fn next() -> Self {
        let seq = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        Self(format!("req-{seq}"))
    }

    /// Borrow the underlying string identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::next()
    }
}

/// Kernel errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KernelError {
    /// No factory registered for the language.
    #[error("no kernel available for language `{0}`")]
    Unsupported(Language),
    /// Operation not allowed in the current status.
    #[error("kernel is {0:?}")]
    InvalidStatus(KernelStatus),
    /// Kernel process failed to start.
    #[error("failed to start kernel: {0}")]
    Startup(String),
    /// Kernel handshake failed or advertised an incompatible API version.
    #[error("kernel handshake failed: {0}")]
    Handshake(String),
    /// Kernel does not implement a requested optional capability.
    #[error("kernel capability is unavailable: {0}")]
    CapabilityUnavailable(&'static str),
    /// Requested Arrow value profile is not supported by the host or adapter.
    #[error("unsupported value profile `{id}` v{version}")]
    UnsupportedValueProfile {
        /// Profile identifier.
        id: String,
        /// Profile version.
        version: u16,
    },
    /// Underlying process or socket transport failed.
    #[error("kernel transport error: {0}")]
    Transport(String),
    /// Kernel sent malformed or unexpected protocol data.
    #[error("kernel protocol error: {0}")]
    Protocol(String),
    /// Kernel operation exceeded its deadline.
    #[error("kernel operation timed out: {0}")]
    Timeout(String),
    /// Active execution was interrupted before completion.
    #[error("kernel execution was interrupted")]
    Interrupted,
    /// Code raised a runtime error inside the kernel.
    #[error("{ename}: {evalue}")]
    Execution {
        /// Error type name.
        ename: String,
        /// Error message.
        evalue: String,
        /// Optional formatted stack frames.
        traceback: Vec<String>,
    },
    /// Value import or export failed across the kernel boundary.
    #[error("kernel value transfer error: {0}")]
    Transfer(String),
}

/// Lifecycle status of a kernel session (aligned with ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KernelStatus {
    /// Not started yet.
    NotStarted,
    /// Launching the kernel process and connecting channels.
    Starting,
    /// Performing API and capability negotiation.
    Handshaking,
    /// Ready for requests.
    Idle,
    /// Executing code.
    Busy,
    /// Interrupt request is in flight.
    Interrupting,
    /// Shutting down and relaunching a fresh kernel process.
    Restarting,
    /// Startup, handshake, or transport failed unexpectedly.
    Failed,
    /// Shut down gracefully.
    Stopped,
}

impl KernelStatus {
    /// Stable lowercase label suitable for domain events and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Starting => "starting",
            Self::Handshaking => "handshaking",
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Interrupting => "interrupting",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Kernel capabilities advertised by a versioned SciRes adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelCapabilities {
    /// Execute code in the kernel.
    pub execute: bool,
    /// Interrupt an active execution.
    pub interrupt: bool,
    /// Shut down the process.
    pub shutdown: bool,
    /// Restart the process and clear its namespace.
    pub restart: bool,
}

impl Default for KernelCapabilities {
    fn default() -> Self {
        Self {
            execute: true,
            interrupt: true,
            shutdown: true,
            restart: true,
        }
    }
}

/// Arrow value profile identifier and version advertised by an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueProfile {
    /// Profile identifier, for example `scires.value`.
    pub id: String,
    /// Profile version.
    pub version: u16,
}

impl ValueProfile {
    /// Create a value profile descriptor.
    pub fn new(id: impl Into<String>, version: u16) -> Self {
        Self {
            id: id.into(),
            version,
        }
    }

    /// Return the v1 SciRes Arrow value profile (`scires.value` v1).
    pub fn scires_v1() -> Self {
        Self::new(VALUE_PROFILE_ID, VALUE_PROFILE_VERSION)
    }
}

/// SciRes kernel API handshake returned by a kernel adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelHandshake {
    /// SciRes API identifier.
    pub api: String,
    /// SciRes API major version.
    pub version: u16,
    /// Runtime and implementation details.
    pub info: KernelInfo,
    /// Operations implemented by this adapter.
    pub capabilities: KernelCapabilities,
    /// Arrow value profiles supported by this adapter.
    pub value_profiles: Vec<ValueProfile>,
}

impl KernelHandshake {
    /// Create a v1 handshake for a kernel implementation with no value profiles.
    pub fn new(info: KernelInfo, capabilities: KernelCapabilities) -> Self {
        Self {
            api: KERNEL_API_ID.to_owned(),
            version: KERNEL_API_VERSION,
            info,
            capabilities,
            value_profiles: Vec::new(),
        }
    }

    /// Attach supported Arrow value profiles to this handshake.
    pub fn with_value_profiles(mut self, profiles: Vec<ValueProfile>) -> Self {
        self.value_profiles = profiles;
        self
    }

    /// Validate that this handshake matches the supported SciRes kernel API and version.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::Handshake`] if [`Self::api`] is not [`KERNEL_API_ID`]
    /// or [`Self::version`] is not [`KERNEL_API_VERSION`].
    pub fn validate(&self) -> Result<(), KernelError> {
        if self.api != KERNEL_API_ID {
            return Err(KernelError::Handshake(format!(
                "unsupported kernel API `{}`; expected `{KERNEL_API_ID}`",
                self.api
            )));
        }
        if self.version != KERNEL_API_VERSION {
            return Err(KernelError::Handshake(format!(
                "unsupported kernel API version {}; expected {KERNEL_API_VERSION}",
                self.version
            )));
        }
        Ok(())
    }

    /// Intersect this adapter's advertised value profiles with those supported by the host.
    pub fn negotiate_profiles(&self, host_supported: &[ValueProfile]) -> Vec<ValueProfile> {
        let mut negotiated = Vec::new();
        for profile in &self.value_profiles {
            if host_supported.contains(profile) && !negotiated.contains(profile) {
                negotiated.push(profile.clone());
            }
        }
        negotiated
    }
}

/// Static description of a kernel implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelInfo {
    /// Language executed.
    pub language: Language,
    /// Implementation name (e.g. "IJulia").
    pub implementation: String,
    /// Language runtime version.
    pub language_version: String,
}

/// Request to execute code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Host-assigned request identifier for output and reply correlation.
    pub request_id: RequestId,
    /// Source code.
    pub code: String,
    /// Do not record in history / don't echo output.
    pub silent: bool,
}

impl ExecuteRequest {
    /// Convenience constructor assigning a fresh [`RequestId`].
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            request_id: RequestId::next(),
            code: code.into(),
            silent: false,
        }
    }

    /// Override the request identifier for deterministic correlation.
    pub fn with_request_id(mut self, request_id: RequestId) -> Self {
        self.request_id = request_id;
        self
    }

    /// Mark whether execution should be silent.
    pub fn with_silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
}

/// Rich output produced by execution (MIME bundle, as in Jupyter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Output {
    /// Stdout text.
    Stdout(String),
    /// Stderr text.
    Stderr(String),
    /// Display data with a MIME type (e.g. `image/png`, `application/vnd.scires.plot+json`).
    Display {
        /// MIME type.
        mime: String,
        /// Payload (base64 for binary).
        data: String,
    },
}

/// Result of a successful execution.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecuteResult {
    /// Request identifier echoed from [`ExecuteRequest::request_id`].
    pub request_id: RequestId,
    /// Monotonic execution counter.
    pub execution_count: u64,
    /// Produced outputs, in order.
    pub outputs: Vec<Output>,
}

/// Thread-safe control handle for out-of-band kernel operations such as
/// interrupting an in-flight execution while [`Kernel::execute`] is running.
pub trait KernelControl: Send + Sync {
    /// Request interruption of the active execution.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::CapabilityUnavailable`] if the adapter cannot
    /// interrupt execution, or [`KernelError::Transport`] if the signal or
    /// control message could not be delivered.
    fn interrupt(&self) -> Result<(), KernelError>;
}

/// Port implemented by every language kernel.
pub trait Kernel: Send {
    /// Static information.
    fn info(&self) -> KernelInfo;
    /// Current status.
    fn status(&self) -> KernelStatus;
    /// Start the kernel (spawn process, connect sockets, perform remote handshake).
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::Startup`], [`KernelError::Handshake`], or
    /// [`KernelError::Transport`] if the kernel cannot be initialized.
    fn start(&mut self) -> Result<(), KernelError>;
    /// Execute code.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::InvalidStatus`] if the kernel is not ready,
    /// [`KernelError::Execution`] if evaluated code raises an error,
    /// [`KernelError::Interrupted`] if cancelled, or transport/protocol/timeout errors.
    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError>;
    /// Interrupt running code.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::CapabilityUnavailable`] if interrupt is unsupported,
    /// or [`KernelError::Transport`] if delivery fails.
    fn interrupt(&mut self) -> Result<(), KernelError>;
    /// Optional thread-safe control handle for concurrent out-of-band interrupt.
    fn control(&self) -> Option<Arc<dyn KernelControl>> {
        None
    }
    /// Snapshot of the kernel's global variables for the variable explorer.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError`] if the kernel cannot be queried.
    fn variables(&mut self) -> Result<Vec<Variable>, KernelError>;
    /// Shut the kernel down gracefully.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::Transport`] or [`KernelError::Timeout`] if shutdown fails.
    fn shutdown(&mut self) -> Result<(), KernelError>;
    /// Restart the kernel and clear its in-memory namespace.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Self::shutdown`] or [`Self::start`].
    fn restart(&mut self) -> Result<(), KernelError> {
        self.shutdown()?;
        self.start()
    }
    /// Advertise the versioned SciRes API capabilities implemented by this adapter.
    fn handshake(&self) -> KernelHandshake {
        KernelHandshake::new(self.info(), KernelCapabilities::default())
    }
    /// Export a named kernel variable encoded as opaque bytes for `profile`.
    ///
    /// Keeping the payload opaque (`Vec<u8>`) keeps Arrow serialization inside
    /// `scires-interchange` and out of the pure `scires-kernel` domain crate.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::UnsupportedValueProfile`] if `profile` is not supported,
    /// or [`KernelError::Transfer`] if the variable does not exist or cannot be encoded.
    fn export_value(&mut self, _name: &str, profile: &ValueProfile) -> Result<Vec<u8>, KernelError> {
        Err(KernelError::UnsupportedValueProfile {
            id: profile.id.clone(),
            version: profile.version,
        })
    }
    /// Import a named value encoded as opaque bytes for `profile` into the kernel.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::UnsupportedValueProfile`] if `profile` is not supported,
    /// or [`KernelError::Transfer`] if the payload cannot be decoded or bound.
    fn import_value(
        &mut self,
        _name: &str,
        profile: &ValueProfile,
        _payload: &[u8],
    ) -> Result<(), KernelError> {
        Err(KernelError::UnsupportedValueProfile {
            id: profile.id.clone(),
            version: profile.version,
        })
    }
}

/// Creates kernels of one language; registered by plugins.
pub trait KernelFactory: Send + Sync {
    /// Language produced.
    fn language(&self) -> Language;
    /// Create a new, not-yet-started kernel.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::Startup`] if the kernel cannot be constructed.
    fn create(&self) -> Result<Box<dyn Kernel>, KernelError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info() -> KernelInfo {
        KernelInfo {
            language: Language::Julia,
            implementation: "IJulia".into(),
            language_version: "1.12.5".into(),
        }
    }

    #[test]
    fn request_ids_are_unique_and_preserved() {
        let first = ExecuteRequest::new("1 + 1");
        let second = ExecuteRequest::new("2 + 2");
        assert_ne!(first.request_id, second.request_id);

        let custom = ExecuteRequest::new("x").with_request_id(RequestId::new("custom-42"));
        assert_eq!(custom.request_id.as_str(), "custom-42");
    }

    #[test]
    fn handshake_validation_rejects_wrong_api_and_version() {
        let valid = KernelHandshake::new(sample_info(), KernelCapabilities::default());
        assert_eq!(valid.validate(), Ok(()));

        let mut bad_api = valid.clone();
        bad_api.api = "other.kernel".into();
        assert!(matches!(bad_api.validate(), Err(KernelError::Handshake(_))));

        let mut bad_version = valid;
        bad_version.version = 2;
        assert!(matches!(
            bad_version.validate(),
            Err(KernelError::Handshake(_))
        ));
    }

    #[test]
    fn handshake_negotiates_profile_intersection() {
        let v1 = ValueProfile::scires_v1();
        let v2 = ValueProfile::new(VALUE_PROFILE_ID, 2);
        let other = ValueProfile::new("other.value", 1);

        let handshake = KernelHandshake::new(sample_info(), KernelCapabilities::default())
            .with_value_profiles(vec![v2.clone(), v1.clone(), v1.clone(), other]);
        let negotiated = handshake.negotiate_profiles(&[v1.clone()]);
        assert_eq!(negotiated, vec![v1]);
    }
}
