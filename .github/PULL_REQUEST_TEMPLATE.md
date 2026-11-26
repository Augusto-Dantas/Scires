## Summary

<!-- Briefly describe what this PR does and why. Use Conventional Commits for the PR title (e.g., `feat(core): ...`, `fix(kernel): ...`, `docs: ...`). -->

## Related Issue

<!-- Link related issue(s), e.g., `Closes #123` -->

## Type of Change

- [ ] `feat`: New feature
- [ ] `fix`: Bug fix
- [ ] `docs`: Documentation only
- [ ] `refactor`: Code refactoring without behavior change
- [ ] `test`: Adding or updating tests
- [ ] `chore`: Build, CI, or maintenance

## Checklist

- [ ] Follows [Conventional Commits](https://www.conventionalcommits.org/) and the [Code Style Guide](docs/STYLE_GUIDE.md)
- [ ] Domain crates (`scires-core`, `scires-kernel`, `scires-debug`, `scires-plugin`, `scires-viz`) remain free of I/O, UI, and runtime-specific code
- [ ] Public items are documented with `rustdoc` (intra-doc links, `# Errors`, and `# Examples` where helpful)
- [ ] Unit tests cover new or changed business rules
- [ ] Significant design changes include an ADR in `docs/adr/` (if applicable)
- [ ] Verified locally:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`
