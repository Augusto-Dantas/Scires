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

**Consequences:** Under the target policy, third-party code is sandboxed by default and
authors can use any language that targets WASM components. Native ABI stability is not
promised. A WIT interface (`scires-plugin.wit`) becomes a versioned public API.

**Implementation status and security boundary:** This is the target architecture, not a
claim that the runtime policy is enforced today. The repository currently parses and
validates manifests and resolves registry entries; it has no plugin loader, `.srp`
installer, WASM host, process supervisor, permission enforcement or trust UI. The
manifest's current runtime default is `native`, so third-party installation must not rely
on that default: the future host must require an explicit safe runtime or an explicit
trust decision. Kernel processes and `process` plugins are distinct boundaries. A
separate process limits crash impact but still has its operating-system user's access;
it is not a sandbox.
