# ADR-0009: Julia reference-kernel boundary

**Status:** Proposed

**Context:** ADR-0002 and ADR-0008 establish separate, long-lived language processes
behind a versioned SciRes kernel API, with Jupyter-compatible messaging where it fits
and Arrow IPC as the initial typed data representation. `scires-kernel` currently
defines a Rust port and mock kernel, while `scires-interchange` implements the v1 Arrow
value profile. Neither crate launches Julia or verifies a Julia/Jupyter adapter. The
Rust/PyArrow fixture verifies the Arrow format boundary only; IJulia/Jupyter and Julia
value interoperability remain untested.

**Proposal:**

1. **Host and process ownership.** The Rust application owns the Julia kernel process
   through a language plugin and `KernelFactory`. Julia runs out of process; its startup,
   crash, and shutdown do not take down the IDE. The host resolves a Julia executable
   and a project environment explicitly, then starts the adapter and waits for a
   successful capability handshake before accepting execution requests. Project-level
   interpreter/environment settings should override user defaults, with a discoverable
   `julia` on `PATH` as a possible fallback. Exact precedence, command-line options,
   startup-file policy, and environment configuration format remain open.
2. **Control plane.** Keep the versioned SciRes API as the compatibility boundary and
   normalize kernel operations into the existing Rust `Kernel` port. Prefer Jupyter
   execution and control semantics where a supported Julia kernel exposes them: execute,
   status, output, error, interrupt, shutdown, and kernel information. A Julia adapter
   may bridge that protocol to SciRes, but must not assume IJulia behavior that has not
   been verified. SciRes API and capability versions remain authoritative; Jupyter
   compatibility alone does not imply support for every SciRes operation.
3. **Capability handshake.** Before execution, exchange the SciRes API version,
   implementation and Julia versions, supported operations, and supported value
   profile identifiers/versions. Negotiate the intersection and reject incompatible
   API or Arrow profile versions before transferring data. Capabilities may include
   execution, interrupt, restart, variable summaries, display MIME types, and debugging;
   advertise only operations the adapter actually implements. The handshake format and
   required versus optional capabilities remain open.
4. **Request correlation and output.** Assign each execution a unique SciRes request
   identifier and map it to the underlying kernel message identifier where available.
   Associate completion, status, stdout, stderr, display data, and errors with that
   request; preserve output order and distinguish execution errors from launch,
   protocol, and transport failures. Send text and MIME metadata through the control
   protocol. Do not place large scientific values or Arrow streams inside JSON control
   messages.
5. **Separate Arrow data plane.** Transfer supported values using Arrow IPC and the
   negotiated `scires.value` profile, independently of execution-control messages.
   Start with copy-based transport. The adapter validates profile/version and value
   shape before use; unsupported or lossy conversions fail explicitly. CSC offsets and
   indices are zero-based Int64 on the wire and must be converted at the Julia boundary
   to Julia's native one-based indexing, with bounds and overflow checks. Do not
   silently densify sparse matrices. The concrete channel/framing for Arrow bytes is
   not selected by this ADR.
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
| Jupyter-compatible kernel semantics behind the SciRes API (**proposed**) | Reuses established execute/status/output/interrupt concepts and aligns with Julia tooling; keeps SciRes versioning and capabilities in control | IJulia behavior, supported versions, transport setup, and Arrow integration must be validated; compatibility may require a bridge |
| Custom framed JSON over stdio, with separate Arrow frames | Simple process supervision and no socket discovery; can tailor messages to SciRes | Reimplements correlation, output channels, interrupt, shutdown, and compatibility semantics; framing and binary-transfer rules become SciRes-owned protocol |
| Custom socket/JSON-RPC protocol | Explicit request/response methods and can support streaming | Still duplicates kernel semantics; endpoint discovery, lifecycle, framing, and compatibility need definition |
| gRPC/Protobuf control and streaming | Strong schemas, generated clients, and bidirectional streaming | Additional toolchain and dependencies; does not provide Jupyter compatibility by itself and is premature before confirming Julia adapter needs |

**Consequences if accepted:** Julia becomes the first concrete reference for implementing
the versioned kernel API and its conformance tests. Later Python and R adapters can reuse
the host-side lifecycle, correlation, capability, and Arrow contracts without implying
that their runtimes share Julia's launch or protocol details. C/C++ task workers remain
out of scope here. This ADR does not select a final transport or claim working Julia
interoperability.

**Validation plan:** Once the proposal is accepted, choose and pin a Julia/IJulia
combination, then verify executable and project-environment selection; startup and
capability negotiation; ordered output, structured errors, request correlation,
interrupt, shutdown, restart, and process-exit handling; and Arrow profile negotiation
with scalar/table, dense, complex, and CSC values. Check zero-based wire CSC conversion
against Julia indexing, reject unsupported profile versions and values, and exercise UI
state transitions for failure and recovery. Record the tested runtime versions and
limitations; do not infer support for other language kernels.

**Highest-priority remaining decision:** Validate whether a supported IJulia/Jupyter
combination can be launched reproducibly in the selected Julia project environment and
provides the required execution, output, interrupt, and shutdown semantics. This
determines whether the adapter can use Jupyter-compatible messaging or needs a small
SciRes-owned framed control protocol.
