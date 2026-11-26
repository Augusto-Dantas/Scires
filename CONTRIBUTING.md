# Contributing

- Branch from `main`: `feat/…`, `fix/…`, `docs/…`.
- Use [Conventional Commits](https://www.conventionalcommits.org/).
- Before pushing: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --workspace`.
- Domain crates (`scires-core`, `-kernel`, `-debug`, `-plugin`, `-viz`) must stay free of I/O,
  UI and runtime-specific code — put those in adapters/plugins.
- Every public item is documented; every business rule has a unit test.
- Significant design changes require an ADR in `docs/adr/`.
