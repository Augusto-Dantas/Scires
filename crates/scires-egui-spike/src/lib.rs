//! Bounded native UI workbench spike built on `eframe` / `egui`, `egui_dock`, and `egui_plot`.
//!
//! This crate validates [`scires_ui`] (`Layout`, `Theme`), [`scires_viz`] (`PlotSpec`),
//! and [`scires_app`] (`App`, `Command`, `Reply`) against the desktop workbench requirements
//! in `docs/UI_UX.md` while keeping every domain crate toolkit-agnostic and `unsafe`-free.
//!
//! # Examples
//!
//! ```
//! use scires_egui_spike::{build_dock_state, dock_tab_ids, theme_to_visuals};
//! use scires_ui::{Layout, Theme};
//!
//! let visuals = theme_to_visuals(&Theme::dark());
//! assert!(visuals.dark_mode);
//!
//! let dock = build_dock_state(&Layout::scientific());
//! let ids = dock_tab_ids(&dock);
//! assert_eq!(ids.len(), 6);
//! assert!(ids.contains(&"editor".to_string()));
//! assert!(ids.contains(&"console".to_string()));
//! ```

mod dock;
mod plot;
mod theme;
mod workbench;

pub use dock::{build_dock_state, dock_tab_ids, DockTab};
pub use plot::{plot_accessibility_summary, render_plot, sample_plot_spec};
pub use theme::{color_to_egui, theme_to_visuals, ThemeMode};
pub use workbench::{ConsoleEntry, ConsoleKind, WorkbenchApp};
