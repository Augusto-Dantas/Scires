# ADR-0006: Plugin runtimes and build model

**Status:** Accepted

**Context:** ADR-0005 defined manifest-based lazy plugins. We must now decide how plugin
*code* is built and isolated, balancing first-party performance with third-party safety
and polyglot authorship (scientists write Julia/Python, not Rust).

**Decision:** Four runtime kinds declared by `runtime` in `scires-plugin.toml`:

1. `declarative` — no code.
2. `native` — Rust crates statically linked; first-party only.
3. `wasm` — WASI Preview 2 components run in wasmtime with capability-based WIT imports;
   the default for third parties.
4. `process` — out-of-process executables over JSON-RPC/stdio, mirroring how kernels
   (ADR-0002) and DAP adapters (ADR-0004) already run.

`entry` is required for `wasm`/`process` and forbidden for `declarative`. Plugins ship as
`.srp` archives. See [PLUGINS.md](../PLUGINS.md).

**Consequences:** Third-party code is sandboxed by default; authors can use any language
that targets WASM components. Native ABI stability is not promised. A WIT interface
(`scires-plugin.wit`) becomes a versioned public API.
