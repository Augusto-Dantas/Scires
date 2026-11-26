# ADR-0001: Record architecture decisions

**Status:** Accepted

We record significant decisions as ADRs (Michael Nygard format) in `docs/adr/`.

# ADR-0002: Out-of-process kernels via Jupyter protocol

**Status:** Accepted

**Context:** We must support Julia, Python, R and C/C++. Embedding interpreters couples
crash domains and versions. Spyder and JupyterLab use out-of-process kernels.

**Decision:** Kernels run as separate processes speaking the Jupyter messaging protocol.
The domain only sees the `Kernel` trait, so in-process kernels remain possible for tests.

**Consequences:** Reuses IJulia, ipykernel, IRkernel, xeus-cling. Adds IPC latency.

# ADR-0003: Apache Arrow for cross-language data exchange

**Status:** Proposed

**Decision:** Large arrays/tables shared between kernels use Arrow IPC / shared memory.
The workspace stores only summaries (`Value`) for the variable explorer.

# ADR-0004: Debug Adapter Protocol

**Status:** Accepted

**Decision:** The debugger is a DAP client. Adapters: Debugger.jl (Julia), debugpy (Python),
vscDebugger (R), lldb-dap/gdb (C/C++).

# ADR-0005: Manifest-based, lazily-activated plugins

**Status:** Accepted

**Decision:** Plugins declare contributions in `scires-plugin.toml` with semver
compatibility and dependencies, activated on events (VS Code model). Native plugins
initially; WASM (wasmtime) sandbox for third-party code later.
