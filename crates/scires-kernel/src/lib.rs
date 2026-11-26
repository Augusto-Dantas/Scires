//! Kernel abstraction (port) and kernel manager.
//!
//! Concrete kernels (Jupyter/ZeroMQ adapters for Julia, Python, R, C/C++) live in
//! plugins and implement [`Kernel`] + [`KernelFactory`]. [`KernelManager`] owns
//! registered factories and lazily starts kernels on first use.
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
//! let res = mgr.execute(&Language::Julia, &ExecuteRequest::new("1 + 1"))?;
//! assert_eq!(res.outputs, vec![Output::Stdout("1 + 1".into())]);
//! # Ok::<(), scires_kernel::KernelError>(())
//! ```

mod manager;
pub mod mock;

pub use manager::KernelManager;

use scires_core::{Language, Variable};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    /// Code raised an error.
    #[error("{ename}: {evalue}")]
    Execution {
        /// Error type name.
        ename: String,
        /// Error message.
        evalue: String,
    },
}

/// Lifecycle status (mirrors Jupyter `status` messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KernelStatus {
    /// Not started yet.
    NotStarted,
    /// Ready for requests.
    Idle,
    /// Executing code.
    Busy,
    /// Shut down.
    Dead,
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
    /// Source code.
    pub code: String,
    /// Do not record in history / don't echo output.
    pub silent: bool,
}

impl ExecuteRequest {
    /// Convenience constructor.
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            silent: false,
        }
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
    /// Monotonic execution counter.
    pub execution_count: u64,
    /// Produced outputs, in order.
    pub outputs: Vec<Output>,
}

/// Port implemented by every language kernel.
pub trait Kernel: Send {
    /// Static information.
    fn info(&self) -> KernelInfo;
    /// Current status.
    fn status(&self) -> KernelStatus;
    /// Start the kernel (spawn process, connect sockets).
    fn start(&mut self) -> Result<(), KernelError>;
    /// Execute code.
    fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError>;
    /// Interrupt running code.
    fn interrupt(&mut self) -> Result<(), KernelError>;
    /// Snapshot of the kernel's global variables for the variable explorer.
    fn variables(&mut self) -> Result<Vec<Variable>, KernelError>;
    /// Shut the kernel down.
    fn shutdown(&mut self) -> Result<(), KernelError>;
}

/// Creates kernels of one language; registered by plugins.
pub trait KernelFactory: Send + Sync {
    /// Language produced.
    fn language(&self) -> Language;
    /// Create a new, not-yet-started kernel.
    fn create(&self) -> Result<Box<dyn Kernel>, KernelError>;
}
