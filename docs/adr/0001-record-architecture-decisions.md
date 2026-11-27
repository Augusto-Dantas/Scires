# ADR-0001: Record architecture decisions

**Status:** Accepted

We record significant decisions as ADRs (Michael Nygard format) in `docs/adr/`.

# ADR-0002: Out-of-process kernels behind a versioned SciRes API

**Status:** Accepted

**Context:** We must support Julia, Python, R and C/C++. Embedding interpreters couples
crash domains and versions. Spyder and JupyterLab use out-of-process kernels.

**Decision:** Kernels run as separate processes behind a versioned SciRes kernel API.
Jupyter-compatible messaging is used where it fits; small control messages use JSON or
Protobuf. The domain only sees the `Kernel` trait, so in-process kernels remain possible
for tests. Compiled C/C++ tasks use isolated worker processes, never arbitrary native
libraries loaded into the IDE.

**Consequences:** Reuses language kernels where they fit the API and accepts IPC latency
in exchange for crash isolation. Protocol compatibility and feature negotiation belong
to the versioned SciRes API, not directly to domain crates. See ADR-0008.

# ADR-0003: Apache Arrow for cross-language data exchange

**Status:** Accepted

**Decision:** Arrow IPC is the preferred initial representation for explicit typed
cross-kernel transfers. Begin with copy-based transport; defer shared-memory buffers
until benchmarks justify them and ownership/lifetime rules are defined. Dense
N-dimensional fixed-width numeric and Boolean arrays preserve shape and type metadata;
tables use supported Arrow primitive columns; complex values use real/imaginary pairs.
The implemented v1 profile uses CSC sparse matrices with Int64 offsets and indices,
zero-based on the wire, and explicit runtime-boundary conversion; do not silently densify.
Values without a defined mapping remain kernel-local unless explicitly converted;
otherwise return an unsupported-value error. The workspace stores `Value` summaries
for the variable explorer, not the transferred payload.

The Rust codec and exact v1 profile are documented in ADR-0008. Julia/R/C++ runtime
adapters, process transport and conversion UX remain open. The Python-only comparison in
`tests/bench_data_exchange.py` is exploratory and does not establish SciRes or Julia
interoperability. See ADR-0008 and proposed ADR-0009.

# ADR-0004: Debug Adapter Protocol

**Status:** Accepted

**Decision:** The debugger is a DAP client. Adapters: Debugger.jl (Julia), debugpy (Python),
vscDebugger (R), lldb-dap/gdb (C/C++).

The repository currently defines debugger domain state and an adapter port; concrete
adapters and kernel integration are not implemented.

# ADR-0005: Manifest-based, lazily-activated plugins

**Status:** Accepted

**Decision:** Plugins declare contributions in `scires-plugin.toml` with semver
compatibility and dependencies, activated on events (VS Code model). Native plugins
initially; WASM (wasmtime) sandbox for third-party code later.
