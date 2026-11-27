# ADR-0007: UI shell — Tauri proposal and Rust-native toolkit recommendation

**Status:** Superseded (Tauri proposal); egui selected as the recommended direction

**Context:** The IDE needs a code editor, dockable panels, rich plots and plugin views.
This ADR originally proposed Tauri 2 with a web front-end. The approved product direction
now requires the IDE UI itself to be Rust-native.

**Original proposal (superseded):** Use **Tauri 2** with a web front-end (Monaco editor,
Plotly/Vega for `PlotSpec`, xterm.js for the console). UI state lives in the pure
`scires-ui` crate (`Layout`, `Panel`, `Theme`) so it is unit-testable and
toolkit-agnostic.

**Current direction:** The IDE UI must be Rust-native. The workbench and interaction
requirements are in [`UI_UX.md`](../UI_UX.md). This comparison recommends egui for the
first native-shell implementation, while leaving final toolkit selection pending a
focused editor, docking, accessibility, and plugin-view spike. Design tokens in
`scires_ui::Theme` remain toolkit-independent.

| Criterion | egui | Slint |
|---|---|---|
| Rust-native fit | Rust immediate-mode UI with native desktop integrations; custom data-heavy panels can stay in Rust | Rust framework with declarative `.slint` UI language and generated bindings |
| SciRes workbench | Flexible for scientific inspectors, plots, tables, and frequently changing kernel state; docking and a production editor need separate integration | Declarative components suit conventional IDE panels and forms; editor, plot, docking, and plugin-view fit still need validation |
| Project integration | MIT license aligns with the workspace's current MIT license; no new UI DSL/code-generation workflow | Open-source applications may use Slint under GPLv3; other licensing options exist, so the implications for SciRes's current MIT licensing need review |
| Main risks | Immediate-mode patterns and accessibility behavior require deliberate design and testing; editor integration is a separate choice | Adds a UI language/tooling workflow; license choice and how native plugin views integrate need early validation |

**Recommendation:** Start with egui for a narrow workbench spike after the kernel/output
and transfer states stabilize. Its Rust-native model, MIT licensing, and ability to build
custom scientific panels are the best fit for the current repository and its proposed
workbench. Do not treat this as proof that egui already supplies a suitable editor,
docking system, or accessible data-grid. Keep the editor component and the plugin-view
contract independent until tested. Revisit Slint if its declarative model and accessibility
prove materially better and the project explicitly accepts a compatible licensing path.

**Consequences:** Tauri/web front-end and its embedded web components are not selected.
Editor, plotting, console and plugin-view components must fit the eventual native toolkit
and extension contract. The UI model remains toolkit-agnostic, and no UI toolkit is yet
an accepted implementation dependency.

**References:** [egui project and MIT license](https://github.com/emilk/egui),
[Slint license terms](https://github.com/slint-ui/slint/blob/master/LICENSE.md), and
[Slint license options](https://slint.dev/pricing).
