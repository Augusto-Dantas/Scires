//! Core domain model for SciRes.
//!
//! This crate is dependency-light and contains no I/O. Every other crate in the
//! workspace builds on these primitives:
//!
//! - [`Language`] identifies supported programming languages (Julia, Python, R, C/C++, or plugin-defined).
//! - [`Value`], [`DType`], and [`Variable`] represent language-neutral variable summaries for the variable explorer.
//! - [`Workspace`] maintains per-language variable namespaces and emits [`DomainEvent`]s.
//! - [`EventBus`] and [`InMemoryEventBus`] decouple domain state changes from UI subscribers.
//!
//! # Examples
//!
//! ```
//! use std::sync::Arc;
//! use scires_core::{InMemoryEventBus, Language, Value, Variable, Workspace};
//!
//! let mut ws = Workspace::new(Arc::new(InMemoryEventBus::new()));
//! ws.set(Variable::new("x", Language::Julia, Value::Int(42)))?;
//! assert_eq!(ws.get(&Language::Julia, "x"), Some(&Value::Int(42)));
//! # Ok::<(), scires_core::WorkspaceError>(())
//! ```

pub mod event;
pub mod language;
pub mod value;
pub mod workspace;

pub use event::{DomainEvent, EventBus, InMemoryEventBus};
pub use language::Language;
pub use value::{DType, Value, Variable};
pub use workspace::{Workspace, WorkspaceError};
