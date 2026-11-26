# Contributing

- Branch from `main`: `feat/…`, `fix/…`, `docs/…`.
- Use [Conventional Commits](https://www.conventionalcommits.org/).
- Follow the [Code Style Guide](docs/STYLE_GUIDE.md).
- Before pushing: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --workspace` and verify `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`.
- Domain crates (`scires-core`, `-kernel`, `-debug`, `-plugin`, `-viz`) must stay free of I/O,
  UI and runtime-specific code — put those in adapters/plugins.
- Every public item is documented with rustdoc (use intra-doc links, `# Errors` on fallible APIs, and `# Examples` doc-tests where helpful); every business rule has a unit test.
- Significant design changes require an ADR in `docs/adr/`.
