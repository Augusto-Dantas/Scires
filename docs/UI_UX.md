# SciRes UI/UX Design Specification

**Status:** Proposed. This is a product and interaction specification, not a UI
implementation or wireframe. egui is the selected recommendation for the first
native-shell spike; the implementation choice, editor, and Slint reconsideration remain
open.

## 1. Product direction and constraints

SciRes is an open-source, Julia-first scientific-computing environment intended as an
alternative to MATLAB, with a Rust-native IDE. Its UI should make interactive scripting,
data inspection, visualization, debugging, and explicit use of other language runtimes
feel like one coherent workbench without implying one shared interpreter.

### Confirmed constraints

- Julia is the primary scripting environment; Python and R support specialized work,
  while C/C++ tasks cover performance-critical and legacy algorithms.
- The IDE and orchestration are Rust-native. egui is the recommended toolkit for an
  initial workbench spike; it is not yet an implementation dependency or final selection
  (ADR-0007).
- Kernels are intended to run in separate processes with per-language namespaces.
  Cross-language values move only through explicit typed transfers.
- The implemented `scires-interchange` v1 profile provides Arrow IPC scalars, tables,
  dense tensors, complex component pairs, and zero-based Int64 CSC sparse matrices.
  Its Rust/PyArrow fixture verifies the format boundary, not Julia/R/C++ adapters.
- `scires-ui` currently models dock areas and theme tokens. The default scientific
  arrangement is Files, Editor, Variables/Plots, and Console/Debugger.
- `scires-viz` defines a declarative plot model and renderer port. `scires-debug` defines
  DAP-oriented domain state and a port. Neither has a concrete runtime/UI integration.

### Assumptions to validate

This proposal assumes a desktop workbench centered on an opened project, interactive
source files plus a console, and explicit user actions to start runtimes and transfer
values. It does not assume a notebook-first product, automatic package installation,
one project per window, or automatic execution when a project opens.

## 2. Target users and primary tasks

The initial design is for:

- Researchers and analysts who want Julia's interactive scientific workflow in an
  approachable IDE, including users moving from MATLAB-like tools.
- Julia users who need to inspect arrays and tables, create plots, and debug scripts.
- Users who occasionally call Python, R, or compiled C/C++ code for an explicit task or
  to reuse a specialized or legacy algorithm.

Their core tasks are to open a project and select its Julia environment; edit and run
Julia code; inspect variables and plots; explicitly transfer a supported value to another
kernel; diagnose errors; and debug a program.

## 3. Information architecture

| Area | Purpose |
|------|---------|
| Project and files | Open/create a project, navigate source and data files, and see project-level runtime/environment configuration |
| Editor | Edit Julia source first; other languages use the same editor with language-aware actions |
| Console and kernels | Choose an available kernel, start/stop/restart it, run code, and review ordered output |
| Variables | Browse per-kernel names and lightweight `Value` summaries such as type, shape and preview |
| Plots | Browse rendered plot outputs; select a renderer without exposing renderer-specific controls in the kernel |
| Transfers | Review explicit source-to-target transfers, negotiated profile, progress, and errors |
| Debugger | Breakpoints, call stack, variables, stepping, and debug-console actions through DAP |
| Environments and plugins | Inspect runtime selection separately from plugin contributions and trust/capability information |

These are proposed user-facing concepts. Which are persistent dock panels, tabs, drawers,
or command-palette actions remains a layout decision.

## 4. Workbench and layout proposal

Use the existing dock model as the initial desktop arrangement, not as a locked layout:

| Dock area | Initial contents |
|-----------|------------------|
| Left | Project/files and data browser; optional environment/plugin views |
| Center | Editor tabs; source location remains the anchor for run and debug actions |
| Right | Variables grouped by language; Plots as a neighboring tab |
| Bottom | Julia console; Debugger and Transfers as neighboring tabs |

A compact top toolbar may expose project selection, Julia environment, kernel status,
Run, Interrupt, and Restart. A status strip should distinguish the active language and
kernel state from the selected project environment. Panels should be resizable,
hideable, and restorable through `scires-ui::Layout`; narrow-window behavior and layout
persistence are open.

The shell must remain Rust-native. Plugin views should use a host-defined native view
contract; the current `ui.view` contribution does not select HTML, webviews, or a
serialized view schema. The existing Figma concepts are illustrative, not a toolkit or
interaction contract.

## 5. Essential flows

### Open a project and select an environment

1. Open a folder or create a project; show detected Julia project files and available
   Julia executables without executing project code.
2. Show which executable and project environment will be used. Offer selection or
   configuration before first start, and keep the chosen values visible afterward.
3. If project trust prompts are adopted, show the reason and scope before running code.
   The trust policy and whether it gates execution remain open; opening a project should
   not itself run code or silently install packages.

Whether SciRes creates a `Project.toml`/`Manifest.toml`, how it chooses among Julia
versions, and how Python/R/C++ environments are represented are open product decisions.
Show runtime environments separately from plugin installation locations.

### Edit and run Julia

1. Edit a `.jl` file and choose Run File, Run Selection, or Run Current Line/Block.
   Display the active Julia executable/environment and kernel state with the action.
2. Start the kernel lazily on the first explicit run. Show `Starting` and `Handshaking`
   before `Ready`; if API/profile negotiation fails, disable execution and show a
   diagnostic rather than falling back silently.
3. Stream stdout and stderr to the console in order, keep structured errors linked to
   source locations where available, and render recognized MIME output in its matching
   panel.
4. While running, expose Interrupt and a clear busy indicator. Restart should explain
   that the kernel's in-memory namespace will be lost.

The exact Julia transport and whether line, cell, selection, and file are all first-class
execution units depend on the Julia adapter and editor decision.

### Inspect variables and plots

- Group variables by owning kernel; show name, runtime type, shape, and a short preview.
  The explorer displays summaries, not implicit copies of full data.
- Selecting an array/table may request a bounded preview. Label the source language and
  avoid rendering a stale preview as if it were the live value.
- Route `application/vnd.scires.plot+json` through a registered `Renderer`; show plot
  errors and missing renderer capability explicitly.
- Keep plot selection independent from kernel execution. The current `PlotSpec` embeds
  Float64 samples; large-data references, downsampling, and renderer resource limits need
  a future contract.

### Explicitly transfer a value to another language

1. Start from a named variable and choose **Send to...**; show its owning kernel and the
   target kernel before any data moves.
2. Preview the value kind, dtype, shape, missing-value behavior, sparse format, and
   approximate transfer size when available. Show the negotiated SciRes profile/version
   and whether the target advertises support.
3. Require an explicit destination name or a clearly shown default. Do not imply that
   the source and destination become a shared variable.
4. Transfer only after validation. Do not silently cast, densify sparse data, drop nulls,
   or send a `Value` preview in place of the payload. If a future explicit conversion is
   offered, show the transformation and require confirmation.
5. Report completion with source, target, destination name, and bytes/status when known.
   On failure, identify whether the value is unsupported, versions are incompatible, or
   the kernel/transport failed; leave the source untouched.

Transfer history, cancellation, retry policy, and size thresholds are open. Cancellation
must not be shown as available until the transport can stop safely.

### Debug a program

Use a DAP-backed debugger surface for breakpoints, call stack, stepping, local variables,
and debug evaluation. Make the selected language, executable, and project environment
visible. When paused, distinguish frame-local variables from the normal kernel explorer.
If the debugger cannot attach, the kernel restarts, or the adapter is unavailable, state
which session was affected and offer a recovery action. Whether Julia debugging attaches
to the interactive kernel or launches a separate debuggee, and whether the console can
run while paused, remain open.

## 6. Kernel, transfer, and error states

| Concern | User-visible states | Feedback and recovery |
|---------|---------------------|-----------------------|
| Kernel | Not started, Starting, Handshaking, Ready, Running, Interrupting, Restarting, Failed, Stopped | Disable Run until Ready; show output while Running; expose Interrupt/Restart only when supported; show launch, handshake, timeout, and exit diagnostics |
| Transfer | Validating, Ready, Transferring, Completed, Failed, Unsupported | Identify source/target and current step; never report success before decode/validation completes; provide retry only when safe |
| Environment | Unselected, Resolving, Ready, Missing/Invalid | Show executable path/version and project environment; provide selection/fix action; do not silently change project files |
| Debugger | Idle, Starting, Running, Paused, Terminated, Failed | Keep debugger state separate from kernel busy/ready status; explain process loss and adapter errors |
| Plot/rendering | Rendering, Displayed, Unsupported, Failed | Preserve console output; identify unsupported MIME or renderer errors without hiding the originating execution |
| Plugin/view | Disabled, Loading, Active, Failed, Untrusted (if trust policy exists) | Show plugin identity and requested capabilities; do not equate process isolation with sandboxing |

`Ready` in the UI corresponds to the kernel API's `Idle` state. Exact timeout behavior,
interrupt escalation, and capability-specific states depend on the adapter contract.

## 7. Keyboard, accessibility, and theming

### Keyboard

Every essential action should be reachable by keyboard, including project open, command
search, Run, Interrupt, Restart, panel navigation, transfer initiation, and debugger
controls. Candidate shortcuts such as `Ctrl+Enter` for Run Selection and a platform-aware
Command Palette shortcut require usability testing; exact defaults, remapping, and
conflict resolution remain open. Preserve normal editor, terminal, and platform text
navigation conventions.

### Accessibility

- Provide visible focus, logical tab order across dock panels, keyboard operation for
  dialogs and tables, and accessible names/status announcements.
- Never communicate kernel or transfer state by color alone; pair color with text and
  icons. Support high-contrast palettes, text scaling, and reduced motion.
- Make variable previews and plot descriptions available in textual/table forms where
  meaningful; do not make chart color the only source of information.
- Keep error messages specific, actionable, and associated with the relevant file,
  kernel, variable, or transfer request.

### Theming

Use the existing `Theme` tokens as the shared source of UI colors. Dark and light
palettes are represented in the model; system-theme following, high contrast, user
customization, and plugin theme contributions need product decisions. Plugin views must
inherit host accessibility and theme conventions rather than styling an isolated shell.

## 8. Open decisions and validation

Open decisions include final toolkit selection and the editor component; notebook/cell
support; project/window model; Julia executable/environment precedence; whether opening
an untrusted project requires an execution trust prompt; exact shortcut defaults;
transfer preview and size policy; plot handling for large arrays; plugin-view schema; and
the Julia debugger's relationship to the interactive kernel.

Before implementing the shell, validate the primary flows with researchers who use
Julia and MATLAB-like environments: open/select an environment, run a script and inspect
an error, inspect an array/table and plot, transfer a supported value to another kernel,
and debug a stopped program. Record task completion, confusing states, and accessibility
issues. Validate that the design still works with a non-Julia kernel and with transfer
or debugger capabilities unavailable. This spec does not claim that those integrations
exist.

## 9. Native toolkit recommendation

ADR-0007 compares egui and Slint against this workbench. The current recommendation is
**egui for the first native-shell spike**: it keeps custom scientific inspectors,
plots, and changing kernel state in Rust, and its MIT license aligns with the repository's
current MIT license. The toolkit-independent `scires-ui` layout and theme model should
remain the source of truth.

Slint offers a declarative component language that may suit conventional IDE panels and
accessibility integrations, but adds a UI DSL/code-generation workflow. Its framework is
available for open-source applications under GPLv3, while the repository is currently
MIT-licensed; review the distribution and licensing implications before choosing it.
Neither toolkit has yet been evaluated with SciRes's editor, large scientific tables,
docking behavior, keyboard navigation, screen readers, or plugin views. The recommendation
is therefore a direction for a bounded prototype, not a claim that those requirements are
already met. See [ADR-0007](adr/0007-ui-shell.md) and its upstream
[egui](https://github.com/emilk/egui) and
[Slint licensing](https://github.com/slint-ui/slint/blob/master/LICENSE.md) references.

## References

- [Architecture](ARCHITECTURE.md), [ADR-0007](adr/0007-ui-shell.md),
  [ADR-0008](adr/0008-polyglot-execution-and-data-transfer.md), and
  [ADR-0009](adr/0009-julia-reference-kernel-boundary.md).
- The existing dock/layout model in `crates/scires-ui` and the current plot/debug
  boundaries in `crates/scires-viz` and `crates/scires-debug`.
