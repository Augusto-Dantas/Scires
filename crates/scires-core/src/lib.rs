//! Core domain model for SciRes.
//!
//! This crate is dependency-light and contains no I/O. Every other crate builds on it.

pub mod event;
pub mod language;
pub mod value;
pub mod workspace;

pub use event::{DomainEvent, EventBus, InMemoryEventBus};
pub use language::Language;
pub use value::{DType, Value, Variable};
pub use workspace::{Workspace, WorkspaceError};
