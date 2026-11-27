# ADR-0010: Direct Rust Jupyter/ZeroMQ transport for the Julia kernel

**Status:** Accepted (implemented)

**Context:** ADR-0009 established the Julia reference-kernel boundary using the Jupyter
messaging protocol behind **SciRes Kernel API v1**, with Apache Arrow IPC on a separate
data plane. As an initial bounded vertical slice, `crates/scires-julia-kernel` spawned a
Python helper (`julia_bridge.py`) that used `jupyter_client` and `pyzmq` to launch IJulia
(running `IJulia.installkernel` into a temporary `JUPYTER_DATA_DIR` on every start) and
relayed requests over blocking stdin/stdout JSON-lines.

ADR-0009 left one highest-priority architectural decision open: whether to retain the
Python `jupyter_client`/`pyzmq` bridge as a supported runtime dependency or replace it
with direct Rust Jupyter/ZeroMQ transport.

Several concrete limitations of the Python bridge now block Phase 2 work:

- **Packaging and portability:** Every user installation would require a compatible Python
  runtime plus `jupyter_client` and `pyzmq` (which bundles or links native `libzmq`) just
  to run a Julia kernel, alongside OS-specific Python executable discovery (`python` vs.
  `python3` vs. virtualenv layouts).
- **Reliability and startup overhead:** Supervising a double-hop process chain
  (Rust → Python → IJulia) adds failure modes, extra startup latency from spawning Python
  and running `IJulia.installkernel`, and coarse error reporting.
- **Concurrent interrupt and request correlation:** The synchronous stdin/stdout bridge
  cannot issue an interrupt while an execution request is in flight, and does not expose
  native multi-channel (`shell`/`iopub`/`control`) correlation or crash detection.
- **Maintenance:** Maintaining a dual-language Rust + Python protocol shim duplicates
  message framing, lifecycle state, and error handling across two ecosystems.

**Decision:**

1. **Replace the Python bridge with direct Rust Jupyter/ZeroMQ transport** implemented
   inside `crates/scires-julia-kernel` over pure-Rust `zeromq` and `tokio`.
2. **Isolate the wire implementation behind an internal `transport` boundary** inside
   `crates/scires-julia-kernel`. Prefer `jupyter-zmq-client` + `jupyter-protocol` (the
   `runtimed` ecosystem) when the workspace/crate MSRV and dependency constraints permit;
   otherwise implement a minimal pure-Rust Jupyter wire layer (`zeromq` + `hmac`/`sha2` +
   `tokio`) behind the same internal trait/module boundary so the public `JuliaKernel` API
   is identical either way.
3. **Launch Julia directly with an ephemeral connection file** (for example,
   `julia --startup-file=no --project=<env> -e "using IJulia; IJulia.run_kernel()" <conn.json>`),
   removing both the Python supervisor process and per-startup `IJulia.installkernel` /
   temporary kernelspec generation.
4. **Support concurrent interrupt and request correlation** using split async `shell`,
   `iopub`, and `control` ZeroMQ channels on `tokio`:
   - Correlate `iopub` outputs (including full MIME `Output::Display` payloads and
     structured `ename`/`evalue` errors) and `shell` replies to SciRes `RequestId` values
     via Jupyter `msg_id` / `parent_header.msg_id`.
   - Expose a thread-safe `KernelControl` handle so callers can issue interrupts while
     `execute` is blocked, sending `interrupt_request` on the `control` channel (supported
     by IJulia) with OS signal / process-termination escalation on timeout.
5. **Keep `scires-kernel` pure and transport-agnostic.** `scires-kernel` must remain free
   of `tokio`, ZeroMQ, I/O, and `arrow-*` dependencies. It defines only the domain
   contract (`Kernel`, `KernelControl`, `RequestId`, lifecycle states, structured
   `KernelError`, handshake validation/capability intersection, and an opaque byte-level
   Arrow transfer port).
6. **Keep `tests/julia_kernel_lifecycle.py` strictly as an external conformance fixture.**
   The Python script remains useful for independently checking IJulia's upstream Jupyter
   protocol behavior, but it is not a runtime dependency of SciRes or
   `scires-julia-kernel`.

**Alternatives considered:**

| Criterion | Pure-Rust `zeromq` + `tokio` via `jupyter-zmq-client` / `jupyter-protocol` | Pure-Rust `zeromq` + `tokio` with minimal wire layer (`hmac`/`sha2`) (**implemented**) | `zmq` crate (`libzmq` C++ FFI) | Retain Python `jupyter_client` / `pyzmq` bridge |
|---|---|---|---|---|
| **Packaging** | Single Rust binary; no Python runtime or system `libzmq` | Single Rust binary; no Python runtime or system `libzmq` | Requires C++ toolchain, `cmake`, or preinstalled `libzmq` | Requires user Python + `jupyter_client` + `pyzmq` |
| **Portability** | High; pure-Rust ZeroMQ compiles cleanly across Linux/macOS/Windows | Highest; 100% pure Rust with no C/asm crypto dependencies | Low; C++ FFI complicates cross-compilation and Windows builds | Low; OS-specific Python discovery and venv/wheels fragility |
| **Reliability** | Direct Rust → Julia supervision; explicit crash→`Failed` detection and timeout escalation | Same direct supervision and failure semantics | Direct supervision, but `!Send` `libzmq` sockets require thread-pinned workarounds | Double-hop Rust → Python → Julia process chain and per-start `installkernel` |
| **Maintenance** | Active upstream (`runtimed` ecosystem) owns Jupyter wire types and signing | Small (~250 LOC) ZMTP/HMAC wire layer owned in-repo; no Python shim | Sporadic upstream maintenance on Rust `libzmq` bindings | High dual-language maintenance across Rust and Python |
| **`unsafe_code = "forbid"`** | Compliant (zero workspace `unsafe`; crypto dep may use `ring`/`aws-lc-rs`) | Compliant (zero workspace `unsafe`; pure-Rust `hmac`/`sha2`) | Compliant in workspace, but wraps heavy C++ FFI in dependency | Compliant in workspace, but delegates runtime to Python/C extensions |
| **MSRV** | Watch-out: `zeromq` 0.7 requires Rust ~1.89 vs. workspace `rust-version = "1.75"`; requires crate-local MSRV bump or fallback | Lower MSRV possible; isolates MSRV impact to `scires-julia-kernel` | Tied to C++ toolchain as well as Rust version | Adds Python version matrix on top of Rust MSRV |
| **Async correlation & concurrent interrupt** | Native `tokio` split `shell`/`iopub`/`control` channels (`interrupt_request` + signal fallback) | Native `tokio` split `shell`/`iopub`/`control` channels (`interrupt_request` + signal fallback) | Awkward; `!Send` sockets resist ergonomic async splitting | Requires inventing a custom multiplexed async stdio protocol over the Python shim |

**Consequences:**

- SciRes eliminates Python as a runtime prerequisite for Julia users, simplifying
  packaging and cross-platform distribution.
- `crates/scires-kernel` stays pure, synchronous/transport-agnostic at the domain
  boundary, and reusable for future Python, R, and C/C++ adapters without leaking ZeroMQ
  or `tokio` into domain crates.
- `crates/scires-julia-kernel` will take on `tokio` and pure-Rust `zeromq` (plus either
  `jupyter-zmq-client`/`jupyter-protocol` or `hmac`/`sha2`), and may declare a higher
  crate-local MSRV than the workspace baseline (`1.75`) if required by `zeromq`.
- Arrow data-plane exchange remains separate from Jupyter JSON control messages (per
  ADR-0008 and ADR-0009) and will be implemented over a dedicated local binary channel in
  a subsequent slice rather than multiplexed into Jupyter JSON frames.

**Implementation status and rollout:**

This ADR is **Accepted (implemented)**. Both the direct Rust Jupyter/ZeroMQ control plane
and the live-kernel loopback Arrow IPC data plane are implemented and live-verified in
`crates/scires-julia-kernel` and wired into `crates/scires-app`:

- **Control plane:** Implemented using Option B — pure-Rust `zeromq 0.7` + `tokio` +
  `hmac`/`sha2` behind internal `protocol` and `transport` modules (zero C/C++/asm FFI
  and zero workspace `unsafe`). `julia_bridge.py` and the Python runtime dependency have
  been removed.
- **Live Arrow data plane:** Implemented via a dedicated loopback TCP binary channel
  (`127.0.0.1` ephemeral port + per-session random authentication token, length-prefixed
  Arrow IPC frames — never embedded in Jupyter JSON) served by an embedded Julia helper
  (`Main.__SciResDataPlane`, loaded dynamically on handshake/restart). The adapter
  verifies the channel with a `ping` and advertises `scires.value` v1 only when `Arrow`
  is available and verified (degrading cleanly to `value_profiles: []` if absent). Both
  `import_value` and `export_value` validate identifiers and decode/verify payloads with
  `scires-interchange`.

Implementation sequence:

1. **Slice 1 — Pure kernel contract evolution** (`scires-kernel`, `scires-app`) —
   **Implemented:** added `RequestId` correlation, thread-safe `KernelControl` for
   concurrent interrupt, ADR-0009 lifecycle states, structured `KernelError` variants,
   handshake validation and `value_profiles` intersection in `KernelManager`, and an
   opaque byte-level Arrow transfer port on `Kernel`, with mock and `App` support.
2. **Slice 2 — Direct Rust Jupyter/ZeroMQ transport** (`scires-julia-kernel`) —
   **Implemented:** removed `julia_bridge.py`; implemented direct Rust `zeromq 0.7` +
   `tokio` + `hmac`/`sha2` transport behind internal `protocol` and `transport` modules,
   ephemeral connection-file launch (`--color=yes --startup-file=no`), `kernel_info`
   handshake, correlated `execute` with full MIME outputs and structured errors/tracebacks,
   concurrent `KernelControl::interrupt()` via `control` channel with signal/kill
   escalation, crash→`Failed` detection, `restart`, and graceful `shutdown`.
3. **Slice 3 — Live-kernel Arrow data-plane exchange** (`scires-julia-kernel`) —
   **Implemented:** added `crates/scires-julia-kernel/julia/scires_dataplane.jl`
   (`Main.__SciResDataPlane`) and `crates/scires-julia-kernel/src/dataplane.rs`
   (`DataPlaneClient`) over an authenticated loopback TCP binary channel carrying
   length-prefixed Arrow IPC streams, advertising `scires.value` v1 only when verified
   and re-initializing cleanly across `restart`.
4. **Slice 4 — Application transfer wiring and docs sync** (`scires-app`, `docs/`) —
   **Implemented:** wired `Command::TransferValue` / `Reply::ValueTransferred` in
   `scires-app` gated by negotiated `scires.value` v1 profiles, and synced architecture
   documentation.

**Validation plan:**

- **Slice 1 (verified):** verified purely in Rust via unit tests and doctests in
  `scires-kernel` and `scires-app` without requiring Julia or Python runtimes.
- **Slice 2 (verified):** live Rust↔IJulia integration test verified on Julia 1.12.5 +
  IJulia 1.34.4 covering launch, `kernel_info` handshake, stdout + `text/plain` MIME
  display, structured execution errors with tracebacks, concurrent
  `KernelControl::interrupt()` on `sleep(30)` → `KernelError::Interrupted`, restart
  clearing the runtime namespace, and graceful shutdown → `Stopped`.
  `tests/julia_kernel_lifecycle.py` remains only as an external IJulia conformance
  fixture.
- **Slice 3 & 4 (verified):** live Rust↔Julia data-plane integration test
  (`crates/scires-julia-kernel/tests/live_arrow_dataplane.rs`) verified on Julia 1.12.5 +
  IJulia 1.34.4 + Arrow 2.8.1 covering `scires.value` v1 handshake advertisement,
  scalars (including `Int32`, `Null`/`missing`, `NaN`, UTF-8, and `Complex128`), tables
  preserving `NaN` vs. `missing`, column-major dense `Int32` and `Complex128` tensors,
  `Float64` CSC sparse matrices (0-based wire ↔ 1-based Julia `SparseMatrixCSC` via
  `LargeList`), explicit rejection of unsupported Julia structs/functions leaving the
  kernel healthy, and `restart` re-initialization. `scires-app` unit tests cover
  `Command::TransferValue` both for negotiated transfers and `UnsupportedValueProfile`
  rejection. Variable snapshots, UI workbench shell, DAP integration, plugin
  loading/sandboxing, and Python/R/C++ adapters remain separate pending phases.
