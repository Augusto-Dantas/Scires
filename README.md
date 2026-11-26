# SciRes

**SciRes** is a scientific computing environment — an open alternative to MATLAB —
centred on the **Julia** language and written in **Rust**.

## Goals

- 🧮 **Julia-first** interactive environment (REPL, editor, workspace/variable explorer).
- 🌐 **Polyglot kernels**: R for data analysis, Python for machine learning, C/C++ for
  existing algorithms and performance-critical code — sharing one workspace.
- 📊 **Easy data visualization** through a declarative, backend-agnostic plot model.
- 🐞 **Spyder-like debugger**: breakpoints, stepping, call stack, variable inspection
  (built on the Debug Adapter Protocol).
- 🧩 **Plug-and-play plugins** (like MATLAB toolboxes / VS Code extensions), declared by a
  `scires-plugin.toml` manifest.

## Status

Phase 1 — **architecture and business logic** (no UI yet). See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and the [ADRs](docs/adr).

## Layout

| Crate | Responsibility |
|-------|----------------|
| `scires-core`   | Domain model: languages, values, workspace, domain events |
| `scires-kernel` | Language-agnostic kernel abstraction & kernel manager |
| `scires-debug`  | Debug session state machine, breakpoints, DAP-style adapter trait |
| `scires-plugin` | Plugin manifest, validation, registry, dependency resolution |
| `scires-viz`    | Declarative plot specification & renderer trait |
| `scires-app`    | Application services / command bus wiring everything together |

## Development

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --open
```

## Documentation

API documentation is generated with **rustdoc** across all workspace crates (including runnable doc-tests verified by `cargo test --workspace`). Build and open the docs locally with:

```sh
cargo doc --no-deps --workspace --open
```

Architecture notes, style conventions, and decision records live in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), [`docs/STYLE_GUIDE.md`](docs/STYLE_GUIDE.md), and [`docs/adr`](docs/adr).
