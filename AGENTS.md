# AGENTS.md

Instructions for AI coding agents working in the **SciRes** repository.

## Project Overview

**SciRes** is a scientific computing environment centred on **Julia** (with polyglot support for Python, R, C/C++), written in **Rust** using a hexagonal (ports-and-adapters) architecture. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design.

## Workspace Layout

| Crate | Role |
|-------|------|
| `crates/scires-core` | Domain model: `Language`, `Value`, `Variable`, `Workspace`, `DomainEvent`, `EventBus` |
| `crates/scires-kernel` | `Kernel` / `KernelFactory` ports, `KernelManager`, and `mock::EchoKernel` |
| `crates/scires-debug` | `DebugSession` state machine, `BreakpointStore`, `DebugAdapter` port |
| `crates/scires-plugin` | `PluginManifest` (`scires-plugin.toml`), `PluginRegistry`, dependency resolution |
| `crates/scires-viz` | Declarative `PlotSpec` and `Renderer` port |
| `crates/scires-app` | Application facade (`App`, `Command`, `Reply`) wiring domain crates together |

## Key Rules & Boundaries

- **Keep domain crates pure**: `scires-core`, `scires-kernel`, `scires-debug`, `scires-plugin`, and `scires-viz` must stay free of I/O, UI, and runtime-specific code.
- **Ports and adapters**: Model external capabilities as traits in domain crates; implement concrete adapters in plugins.
- **No `unsafe`**: `unsafe_code = "forbid"` is enforced workspace-wide in [`Cargo.toml`](Cargo.toml).
- **Follow the style guide**: Adhere to [`docs/STYLE_GUIDE.md`](docs/STYLE_GUIDE.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md).
- **Document with `rustdoc`**: Every crate has `//!` crate-level docs with runnable `# Examples`, and every public item has `///` docs (`# Errors` on fallible methods, intra-doc links). `missing_docs` and `[workspace.lints.rustdoc]` are enabled workspace-wide.
- **Test business rules**: Every business rule and validation path must have a unit test in `#[cfg(test)] mod tests`.
- **ADRs**: Significant architectural changes require an ADR in [`docs/adr/`](docs/adr).

## Verification Commands

Run these before finishing any code or documentation change:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
```

## Git & Commits

- Branch from `main`: `feat/…`, `fix/…`, `docs/…`.
- Use [Conventional Commits](https://www.conventionalcommits.org/).
