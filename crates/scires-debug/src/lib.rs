//! Debugger domain, modeled on Spyder's debugger and the Debug Adapter Protocol (DAP).
//!
//! - [`BreakpointStore`] keeps breakpoints independent of any session (persisted per project).
//! - [`DebugAdapter`] is the port each language implements (`Debugger.jl`, `debugpy`, `lldb-dap`, ...).
//! - [`DebugSession`] enforces a valid [`DebugState`] machine on top of an adapter.
//!
//! # Examples
//!
//! ```
//! use std::path::Path;
//! use scires_debug::BreakpointStore;
//!
//! let mut store = BreakpointStore::new();
//! assert!(store.toggle("main.jl", 10)?);
//! assert_eq!(store.active_for(Path::new("main.jl")).len(), 1);
//! # Ok::<(), scires_debug::DebugError>(())
//! ```

mod breakpoints;
mod session;

pub use breakpoints::{Breakpoint, BreakpointId, BreakpointStore};
pub use session::{DebugSession, DebugState, StopReason};

use scires_core::Variable;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Debugger errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DebugError {
    /// Command invalid for current state.
    #[error("cannot `{command}` while {state:?}")]
    InvalidTransition {
        /// Attempted command.
        command: &'static str,
        /// Current state.
        state: DebugState,
    },
    /// Invalid breakpoint definition.
    #[error("invalid breakpoint: {0}")]
    InvalidBreakpoint(String),
    /// Error reported by the adapter.
    #[error("adapter error: {0}")]
    Adapter(String),
}

/// A frame of the call stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackFrame {
    /// Frame id (adapter-specific).
    pub id: u64,
    /// Function name.
    pub name: String,
    /// Source file.
    pub source: Option<PathBuf>,
    /// 1-based line.
    pub line: u32,
}

/// Port implemented per language. Mirrors a subset of DAP requests.
pub trait DebugAdapter: Send {
    /// Launch `program` (a script path) under the debugger.
    fn launch(&mut self, program: &std::path::Path) -> Result<(), DebugError>;
    /// Replace all breakpoints of a source file.
    fn set_breakpoints(&mut self, source: &std::path::Path, bps: &[Breakpoint])
        -> Result<(), DebugError>;
    /// Resume execution.
    fn resume(&mut self) -> Result<(), DebugError>;
    /// Pause execution.
    fn pause(&mut self) -> Result<(), DebugError>;
    /// Step over the current line.
    fn step_over(&mut self) -> Result<(), DebugError>;
    /// Step into a call.
    fn step_in(&mut self) -> Result<(), DebugError>;
    /// Step out of the current function.
    fn step_out(&mut self) -> Result<(), DebugError>;
    /// Current call stack, innermost first.
    fn stack_trace(&mut self) -> Result<Vec<StackFrame>, DebugError>;
    /// Local variables of a frame (Spyder's variable explorer while debugging).
    fn variables(&mut self, frame_id: u64) -> Result<Vec<Variable>, DebugError>;
    /// Evaluate an expression in a frame (debug console / watch).
    fn evaluate(&mut self, expr: &str, frame_id: u64) -> Result<String, DebugError>;
    /// Terminate the debuggee.
    fn terminate(&mut self) -> Result<(), DebugError>;
}
