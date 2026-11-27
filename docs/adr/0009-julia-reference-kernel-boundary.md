# ADR-0009: Julia reference-kernel boundary

**Status:** Accepted for a Julia reference vertical slice; production adapter incomplete

**Context:** ADR-0002 and ADR-0008 establish separate, long-lived language processes
behind a versioned SciRes kernel API, with Jupyter-compatible messaging where it fits
and Arrow IPC as the initial typed data representation. `scires-kernel` currently
defines a Rust port and mock kernel, while `scires-interchange` implements the v1 Arrow
value profile. The Julia test environment verifies IJulia/Jupyter lifecycle behavior
and Rust↔Julia Arrow IPC for scalar, table, dense tensor and CSC values. The new
`scires-julia-kernel` crate now implements a direct Rust Jupyter/ZeroMQ supervisor
(replacing the earlier Python bridge per [ADR-0010](0010-direct-rust-julia-transport.md))
and versioned local handshake; live-kernel Arrow data-plane transfer and remote SciRes
handshake exchange remain unimplemented.

**Decision and implementation boundary:**

1. **Host and process ownership.** The Rust application owns the Julia kernel process
   through a language adapter registered by a plugin. Julia runs out of process; its
   startup, crash and shutdown must not take down the IDE. The adapter resolves a Julia
   executable and project environment explicitly, starts the kernel, and reports
   readiness only after Jupyter kernel-info and the local adapter handshake succeed.
   The test kernelspec selects `tests/julia/Project.toml` with `--project` and disables
   startup files; this validates that mechanism, not the precedence policy for user
   projects. Project-level selection, user defaults and PATH discovery still need a
   product-level precedence rule.
2. **Control plane decision.** Use the Jupyter messaging protocol for Julia
   execution/control, normalized behind **SciRes Kernel API v1**, rather than inventing
   a second Julia-specific wire protocol. On Julia 1.13.2, IJulia 1.34.4 was exercised
   through Jupyter Client 8.6.3 for launch, project selection, execute/result/stdout,
   execution errors, interrupt, graceful shutdown and a fresh restart. Arrow.jl 2.8.1
   was used for the Rust↔Julia fixtures. Per
   [ADR-0010](0010-direct-rust-julia-transport.md), `crates/scires-julia-kernel` now
   replaces the earlier Python `jupyter_client` bridge with direct Rust Jupyter/ZeroMQ
   transport over pure-Rust `zeromq` + `tokio`, launching IJulia directly with an
   ephemeral connection file and advertising `interrupt: true` via the thread-safe
   `KernelControl` handle. Jupyter message IDs and parent headers provide underlying
   correlation and output association. Keep Jupyter as an adapter choice, not a
   requirement for every future language kernel.
3. **Versioned API and capability negotiation.** Define the SciRes-facing handshake as
   an adapter-to-host contract, separate from the Jupyter protocol version and from the
   Arrow value-profile version:

   ```json
   {
     "api": "scires.kernel",
     "version": 1,
     "info": {
       "language": "julia",
       "implementation": "IJulia",
       "language_version": "1.13.2"
     },
     "capabilities": {
       "execute": true,
       "interrupt": true,
       "shutdown": true,
       "restart": true
     },
     "value_profiles": []
   }
   ```

   The adapter reports only capabilities it can actually perform. The host rejects an
   unsupported API version, intersects optional capabilities, and separately rejects
   unsupported value-profile versions before transfer. `restart` is a host lifecycle
   action (shutdown, relaunch, repeat handshake), not an assumed kernel capability.
   Variable snapshots and DAP debugging are optional and must remain absent until
   implemented. The Rust adapter returns this v1 handshake locally; it is not exchanged
   remotely. It advertises no Arrow profile until a live value-transfer path exists.
4. **Request correlation and output.** The host assigns each execution a unique SciRes
   request ID and maps it to the Jupyter `msg_id`; IOPub outputs and the shell reply are
   associated using `parent_header.msg_id`. Preserve per-request output order and
   distinguish execution errors from launch, protocol and transport failures. Send text
   and MIME metadata through Jupyter messages; never put large scientific payloads or
   Arrow streams inside JSON control messages.
5. **Separate Arrow data plane.** Transfer supported values using Arrow IPC and the
   negotiated `scires.value` profile, independently of execution-control messages.
   Start with copy-based transport. The adapter validates profile/version and value
   shape before use; unsupported or lossy conversions fail explicitly. Rust↔Julia tests
   cover scalar, table (valid NaN versus null), dense Int32 tensor metadata and Float64
   CSC round-trips. The CSC fixture confirms zero-based Int64 wire offsets/indices
   convert to Julia's one-based `SparseMatrixCSC` indexing; Julia Arrow writes use
   LargeList/64-bit offsets for the sparse columns. Complex values, other dtypes, large
   payloads and transport of Arrow bytes between a Rust host and a live kernel remain
   untested. The interoperability fixture passes Arrow IPC stream files between Rust
   and Julia; it is not a live-kernel data-plane transport test. Do not silently densify
   sparse matrices. Copy-based IPC streams remain the initial transport choice.
6. **Lifecycle and failure handling.** Model the UI-visible lifecycle as
   `NotStarted → Starting → Handshaking → Idle ↔ Busy`, with `Interrupting`,
   `Restarting`, `Failed`, and `Stopped` transitions. Failed startup or handshake
   reports a diagnostic and leaves execution disabled. A successful graceful shutdown
   ends in `Stopped`; unexpected process exit or protocol loss ends in `Failed`.
   Restart launches a fresh process, repeats negotiation, and clears that kernel's
   runtime namespace; warn that variables and in-memory state will be lost. Map
   interrupt to the kernel's interrupt operation where supported. Request deadlines,
   interrupt grace period, escalation to process termination, and recovery after an
   unresponsive interrupt remain open.
7. **Debugging boundary.** Keep debugging on the Debug Adapter Protocol boundary from
   ADR-0004 rather than embedding debugger commands in execution messages. A Julia DAP
   adapter may attach to or launch the selected Julia process; the exact relationship
   between the debugger, kernel lifecycle, and execution environment remains open.
   Kernel exit/restart must end or invalidate its debug session.
8. **User-visible behavior.** Show Julia as `Not started`, `Starting`, `Ready`, `Running`,
   `Interrupting`, `Restarting`, `Failed`, or `Stopped` in the console/kernel status
   UI. Enable execution only when ready; expose Interrupt while running, Restart when
   the kernel is active or failed, and environment selection before startup. Surface
   stdout/stderr and structured execution errors in the console; expose launch,
   handshake, and protocol diagnostics with a retry/restart action. Confirm before
   restart discards a live namespace. Exact wording, timeout presentation, and whether
   environment selection is project- or user-scoped remain open.

**Alternatives considered:**

| Shape | Advantages | Costs / concerns |
|---|---|---|
| Jupyter messaging behind the SciRes API (**selected for Julia**) | Reuses established execute/status/output/interrupt/shutdown messages; direct Rust ZeroMQ transport and concurrent `KernelControl::interrupt` are implemented and verified against IJulia (ADR-0010) | Live Arrow data-plane transfer (`value_profiles: []`), variable snapshots, remote SciRes handshake exchange, and production environment policy remain unimplemented |
| Custom framed JSON over stdio, with separate Arrow frames | Simple process supervision and no socket discovery; can tailor messages to SciRes | Reimplements correlation, output channels, interrupt, shutdown, and compatibility semantics; framing and binary-transfer rules become SciRes-owned protocol |
| Custom socket/JSON-RPC protocol | Explicit request/response methods and can support streaming | Still duplicates kernel semantics; endpoint discovery, lifecycle, framing, and compatibility need definition |
| gRPC/Protobuf control and streaming | Strong schemas, generated clients, and bidirectional streaming | Additional toolchain and dependencies; does not provide Jupyter compatibility by itself and is premature before confirming Julia adapter needs |

**Consequences:** Julia becomes the first reference for implementing the
versioned kernel API and conformance tests. The repository includes a pinned Julia test
environment, external lifecycle conformance script, and bidirectional file-based
Rust↔Julia Arrow fixtures (`tests/julia/Project.toml`, `tests/julia/Manifest.toml`,
`tests/julia_kernel_lifecycle.py`, and `tests/julia_arrow_interop.py`). Per ADR-0010,
`crates/scires-julia-kernel` now uses direct Rust Jupyter/ZeroMQ transport without a
Python bridge; however, live-kernel Arrow data-plane transfer (`value_profiles: []`),
variable snapshots, remote SciRes handshake exchange, and full profile interoperability
remain unimplemented. Later Python and R adapters may reuse host-side lifecycle,
correlation, capability and Arrow contracts without implying that their runtimes share
Julia's launch or protocol details. C/C++ task workers remain out of scope.

**Security boundary:** A child process isolates crashes but inherits the launching
user's operating-system permissions unless an operating-system sandbox is separately
implemented. Opening a project must not silently execute code or alter its Julia
environment. Whether untrusted projects require an explicit trust action before kernel
execution, what environment mutations require confirmation, and whether resource limits
or OS-level confinement are in scope remain open. Plugin trust and WASM/process-plugin
permissions are separate concerns covered by ADR-0006 and `PLUGINS.md`.

**Validation plan:** Direct Rust Jupyter/ZeroMQ control-plane behavior (startup,
`kernel_info` handshake, stdout/MIME execution output, structured errors with tracebacks,
concurrent `KernelControl::interrupt`, restart clearing namespace, and graceful shutdown)
has been verified against live Julia 1.12.5 + IJulia 1.34.4 in `crates/scires-julia-kernel`.
`tests/julia_kernel_lifecycle.py` remains as an external IJulia conformance fixture, and
`tests/julia_arrow_interop.py` covers file-based Rust↔Julia Arrow IPC for the listed
payloads only (not live-kernel Arrow transfer). Next, implement and test live-kernel
Arrow data-plane transport (`scires.value` v1), remote API/profile negotiation, variable
snapshots, and UI state transitions, and extend interoperability coverage to complex
values, supported temporal/binary/table types, and larger data. Do not infer Julia
support for other versions or any Python/R/C++ kernel.

**Highest-priority remaining decision (resolved by ADR-0010; control plane implemented):**
[ADR-0010](0010-direct-rust-julia-transport.md) replaced the Python Jupyter-client bridge
with direct Rust Jupyter/ZeroMQ transport over pure-Rust `zeromq` + `tokio` inside
`crates/scires-julia-kernel` while keeping `scires-kernel` pure and retaining
`tests/julia_kernel_lifecycle.py` only as an external conformance fixture. The pure
kernel contract evolution (`RequestId`, `KernelControl`, lifecycle states, handshake
validation, and opaque Arrow transfer port) and direct Rust ZeroMQ control plane are now
implemented. Live-kernel Arrow data-plane transfer (`value_profiles: []`), variable
snapshots, and remote SciRes handshake exchange remain pending (Slice 3+).
