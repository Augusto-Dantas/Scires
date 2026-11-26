//! Framework-agnostic UI model for SciRes.
//!
//! This crate holds the *state* of the IDE shell — which panels exist, where they are
//! docked, and which design tokens (colours, spacing) the front-end should use — without
//! depending on any GUI toolkit. The Tauri front-end (ADR-0007) renders this model;
//! plugins add panels via `contributes.views` in their manifest.
//!
//! # Examples
//!
//! ```
//! use scires_ui::{DockArea, Layout, Theme};
//!
//! let mut layout = Layout::scientific();
//! assert!(layout.panel("console").is_some());
//!
//! layout.move_panel("plots", DockArea::Bottom)?;
//! assert!(layout.panels_in(DockArea::Bottom).any(|p| p.id == "plots"));
//!
//! assert_eq!(Theme::dark().palette.accent.to_hex(), "#4F8CFF");
//! # Ok::<(), scires_ui::UiError>(())
//! ```

mod layout;
mod theme;

pub use layout::{DockArea, Layout, Panel, UiError};
pub use theme::{Color, Palette, Theme};
