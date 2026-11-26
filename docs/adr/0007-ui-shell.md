# ADR-0007: UI shell — Tauri front-end over a pure UI model

**Status:** Proposed

**Context:** The IDE needs a code editor, dockable panels, rich plots and plugin views.
Candidates: egui (pure Rust, immediate mode), Tauri (Rust core + web front-end), Slint.

**Decision:** Use **Tauri 2** with a web front-end (Monaco editor, Plotly/Vega for
`PlotSpec`, xterm.js for the console). UI *state* lives in the pure `scires-ui` crate
(`Layout`, `Panel`, `Theme`) so it is unit-testable and toolkit-agnostic. Design tokens in
`scires_ui::Theme` are the source of truth and mirrored in the Figma prototype.

**Consequences:** Mature editor/plotting components and HTML-based plugin views; requires
a JS toolchain for the front-end. egui remains viable for a lightweight mode since the
model is toolkit-independent.
