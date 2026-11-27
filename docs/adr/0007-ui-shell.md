# ADR-0007: UI shell — Tauri proposal (not selected)

**Status:** Superseded

**Context:** The IDE needs a code editor, dockable panels, rich plots and plugin views.
This ADR originally proposed Tauri 2 with a web front-end. The approved product direction
now requires the IDE UI itself to be Rust-native.

**Original proposal (superseded):** Use **Tauri 2** with a web front-end (Monaco editor,
Plotly/Vega for `PlotSpec`, xterm.js for the console). UI state lives in the pure
`scires-ui` crate (`Layout`, `Panel`, `Theme`) so it is unit-testable and
toolkit-agnostic.

**Current direction:** The IDE UI must be Rust-native. egui and Slint remain candidates;
the specific toolkit and component strategy have not been selected (ADR-0008). Design
tokens in `scires_ui::Theme` remain the source of truth and are mirrored in the Figma
prototype.

**Consequences:** Tauri/web front-end and its embedded web components are not selected.
Editor, plotting, console and plugin-view components must fit the eventual native toolkit
and extension contract.
