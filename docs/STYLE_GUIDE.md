# SciRes Code Style Guide

This guide defines the coding, architecture, documentation, and testing conventions for the SciRes Rust workspace.

## 1. Toolchain, Formatting & Lints

- **Edition & MSRV**: Rust 2021, minimum supported Rust version `1.75` (configured in [`Cargo.toml`](../Cargo.toml)).
- **Formatting**: All code must be formatted with `rustfmt` (`cargo fmt --all`). CI enforces `cargo fmt --all -- --check`.
- **Clippy**: All targets must pass `cargo clippy --workspace --all-targets -- -D warnings`.
- **Safety**: `unsafe_code = "forbid"` is enabled workspace-wide in [`Cargo.toml`](../Cargo.toml). Do not introduce `unsafe` blocks.

## 2. Architecture & Crate Boundaries

SciRes follows a hexagonal (ports-and-adapters) architecture (see [`docs/ARCHITECTURE.md`](ARCHITECTURE.md)):

- **Keep domain crates pure**: `scires-core`, `scires-kernel`, `scires-debug`, `scires-plugin`, and `scires-viz` must stay free of I/O, UI, and runtime-specific code.
- **Model boundaries as traits (ports)**: External capabilities are abstracted behind traits such as `Kernel`, `KernelFactory`, `DebugAdapter`, `EventBus`, `Renderer`, and `Plugin`. Concrete implementations live in adapters/plugins.
- **Route UI interactions through `scires-app`**: Consumers interact with the application facade (`App::dispatch(Command)`) and subscribe to `DomainEvent`s rather than mutating domain crates directly.

## 3. Naming & API Conventions

- **Identifiers**: Follow standard Rust naming:
  - `UpperCamelCase` for types, traits, and enum variants (`PlotSpec`, `DebugSession`, `KernelStatus::NotStarted`).
  - `snake_case` for modules, functions, methods, and variables (`step_over`, `active_for`).
  - `SCREAMING_SNAKE_CASE` for constants (`PLOT_MIME`, `VERSION`).
- **Constructors & Conversions**:
  - Use `new(...)` for primary constructors and `Default` when a sensible zero-argument default exists.
  - Use `from_*` for fallible or format-specific constructors (`Language::from_extension`, `PluginManifest::from_toml`, `PlotSpec::from_json`).
- **Builders**: Use fluent value-returning builders for declarative specifications (e.g. `PlotSpec::plot(...).title(...).labels(...).label(...)`).
- **Error Handling**:
  - Define a dedicated error enum per crate using `thiserror::Error` (`WorkspaceError`, `KernelError`, `DebugError`, `PluginError`, `VizError`, `AppError`).
  - Return `Result<T, E>` from fallible operations; avoid `.unwrap()` or `.expect(...)` in library code except for invariants guaranteed by construction (e.g. parsing `CARGO_PKG_VERSION`).

## 4. Documentation (`rustdoc`)

Workspace lints (`missing_docs = "warn"` and `[workspace.lints.rustdoc]`) enforce documentation quality across all crates:

- **Crate-level docs (`//!`)**: Every `lib.rs` must start with crate-level documentation summarizing the crate's role, key types, and at least one runnable `# Examples` doc-test.
- **Public items (`///`)**: Document every public module, struct, enum, variant, field, trait, constant, type alias, and function.
- **Structured sections**:
  - Add `# Errors` to fallible public functions/methods explaining which error variants can be returned and why.
  - Add `# Examples` with compilable doc-tests on key entry points.
- **Intra-doc links**: Prefer Rust intra-doc links (e.g. `` [`Workspace`] ``, `` [`WorkspaceError::InvalidName`] ``) over plain backticks when referencing Rust items.
- **Verification**: Run `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` before pushing.

## 5. Testing

- **Unit tests**: Place unit tests in a `#[cfg(test)] mod tests` block at the bottom of the module under test. Every business rule and validation path should have a unit test.
- **Test doubles**: Use in-memory fakes/mocks for ports (e.g. `scires_kernel::mock::EchoKernelFactory` or `InMemoryEventBus`).
- **Doc-tests**: Keep `# Examples` doc-tests fast and self-contained; they are executed automatically by `cargo test --workspace`.

## 6. Commits & Pull Requests

- Branch from `main` using `feat/…`, `fix/…`, or `docs/…`.
- Follow [Conventional Commits](https://www.conventionalcommits.org/).
- Record significant architectural changes as ADRs in [`docs/adr`](adr).
