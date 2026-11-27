# ADR-0007: UI shell — Rust-native toolkit decision (egui confirmed; Tauri superseded)

**Status:** Accepted (egui confirmed via bounded spike; Tauri superseded)

**Context:** The IDE needs a code editor, dockable panels, rich scientific plots,
variable/debug inspectors, and plugin views. This ADR originally proposed Tauri 2 with a
web front-end. The approved product direction requires the IDE UI itself to be
Rust-native, with toolkit-independent layout and theme state in `crates/scires-ui`.

**Original proposal (superseded):** Use **Tauri 2** with a web front-end (Monaco editor,
Plotly/Vega for `PlotSpec`, xterm.js for the console). Tauri and embedded webviews are
not selected.

**Evaluation (egui vs. Slint):**

| Criterion | egui (**selected**) | Slint |
|---|---|---|
| **Docking** | `egui_dock` (`0.12.0`, MIT) maps `scires_ui::Layout::scientific()` cleanly to a 4-region split tree with tab switching and visibility toggling; bidirectional sync/persistence back into `scires_ui::Layout` needs a reconciliation layer | No standard IDE docking widget out of the box; split/tab docking must be built or integrated separately |
| **Editor integration** | Immediate-mode `egui::TextEdit` works only for a spike (lacks syntax highlighting, LSP, multi-cursor, code folding, soft-wrap gutter alignment, and large-file scalability); production Julia editor must be a decoupled Rust component | Built-in text inputs likewise do not provide a full IDE code editor; requires a decoupled custom editor component |
| **Plots** | `egui_plot` (`0.27.2`, MIT/Apache-2.0) renders `PlotSpec` `Line`, `Scatter`, and `Bar` directly with interactive zoom/pan/legend and paired textual summaries; `Heatmap` requires `PlotImage` texture upload + colourbar | No built-in scientific plotting crate; requires custom canvas/raster integration |
| **Debugging** | `Workspace` variable grid, `DebugState`, and breakpoint panels wire directly to `scires_app::App` + `EventBus` in pure Rust; large tables will need `egui_extras::TableBuilder` virtualization | Declarative panels suit structured forms and debug trees, with Rust state bridged via generated bindings |
| **Accessibility** | `eframe` integrates `AccessKit` (`accesskit_unix` + `atspi` on Linux) and emits non-empty tree updates headlessly; custom widgets (plots, editor gutters, dock splitters) still require deliberate accessible labels, tab-order checks, and live screen-reader testing (Orca/NVDA/VoiceOver) | First-class AccessKit support via declarative widget tree; custom scientific canvases still require explicit accessibility hooks |
| **Platform support** | Native Linux/macOS/Windows via `eframe`/`glow`/`winit`; compiled cleanly on Linux with **zero `pkg-config` and zero C/GUI `-dev` headers** (dynamic X11/Wayland/EGL/GLX loading) | Cross-platform Linux/macOS/Windows via native/Skia/FemtoVG backends; adds `.slint` compiler build step |
| **Licensing** | **MIT OR Apache-2.0** (`eframe`, `egui`, `egui_plot`) and **MIT** (`egui_dock`), matching SciRes's MIT license with zero copyleft or commercial friction | **GPLv3** for open-source applications or commercial license, creating licensing friction with SciRes's MIT license |
| **Maintenance** | Pure Rust with no secondary UI DSL or codegen pipeline; active upstream (`emilk/egui`) and straightforward Cargo dependency management | Active upstream, but introduces a `.slint` DSL/codegen workflow alongside Rust |

**Decision:** Adopt **egui** (`eframe` + `egui_dock` + `egui_plot`) as the Rust-native UI
shell toolkit for SciRes, while keeping `crates/scires-ui` (`Layout`, `Panel`, `Theme`),
`crates/scires-viz`, `crates/scires-core`, and `crates/scires-kernel` completely pure and
toolkit-agnostic.

**Bounded spike evidence (`crates/scires-egui-spike`):**

A bounded prototype in `crates/scires-egui-spike` validated egui against the SciRes
architecture without modifying any domain crate:

- **Dependencies and workspace alignment:** `eframe 0.27.2` (`default-features = false`,
  features: `accesskit`, `default_fonts`, `glow`, `wayland`, `x11`), `egui_dock 0.12.0`,
  and `egui_plot 0.27.2`, aligned with workspace `rust-version = "1.75"`, compiled on
  `rustc 1.93.1` under `unsafe_code = "forbid"`.
- **Build footprint and portability:** Compiled cleanly on Linux with no `pkg-config` and
  no C/GUI `-dev` headers; cold dev build took `1m 03s` (333 newly locked crates) and
  incremental rebuilds took `~3.5s` with zero compiler or rustdoc warnings.
- **Headless verification:** `9/9` unit tests and `1/1` doc-test passed headlessly
  without a display server, verifying `Theme::dark()`/`light()` token mapping, 6-panel
  `Layout::scientific()` docking and visibility toggles, `PlotSpec` rendering plus
  textual accessibility summaries, non-empty `AccessKit` tree updates inside
  `egui::Context::run`, and `scires_app::App` wiring (execution, workspace, console,
  restart, and breakpoints).

**Confirmed gaps and caveats for Phase 4+:**

Confirming egui as the shell toolkit does **not** mean the full workbench is complete.
The following gaps were explicitly identified by the spike and remain open for Phase 4+:

1. **Decoupled production Julia editor:** `egui::TextEdit` is spike-only. Production
   editing needs a decoupled component (e.g., rope-backed custom canvas or dedicated Rust
   text engine) supporting tree-sitter/syntect Julia syntax highlighting, LSP
   completion/diagnostics/hover, multi-cursor/rectangular selection, code folding/bracket
   matching, proper soft-wrap gutter alignment, and large-file scalability.
2. **Bidirectional dock sync and persistence:** `egui_dock` owns its split-ratio tree; a
   reconciliation layer is needed to sync drag-reordering/tab closing back into
   `scires_ui::Layout` and persist layouts.
3. **Heatmaps and large plots:** `LayerData::Heatmap` has no native `egui_plot` raster
   primitive (requires `egui_plot::PlotImage` texture upload + colourbar), and
   large-array downsampling/GPU instancing remains an open contract.
4. **Virtualized scientific tables:** Large variable/data tables must use virtualized
   rows (`egui_extras::TableBuilder`) rather than `egui::Grid`.
5. **Live accessibility verification:** Headless AccessKit emission is verified, but
   custom widgets (plots, editor gutters, dock splitters) still require deliberate
   accessible labels, logical tab-order checks, and live screen-reader testing
   (Orca/NVDA/VoiceOver), pairing icons and text so state never relies on colour alone.
6. **Native plugin-view contract:** The host-defined native view contract for plugin
   contributions remains open.

**Consequences:** Tauri/web front-ends are superseded. SciRes proceeds into Phase 4 on
egui with MIT-compatible pure-Rust dependencies, while keeping `scires-ui` pure and
decoupling the production code editor and plugin-view contract from `egui::TextEdit`.

**References:** [egui project and MIT license](https://github.com/emilk/egui),
[egui_dock](https://github.com/Adanos020/egui_dock),
[Slint license terms](https://github.com/slint-ui/slint/blob/master/LICENSE.md), and
[Slint license options](https://slint.dev/pricing).
