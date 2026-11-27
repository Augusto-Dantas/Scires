# ADR-0008: Polyglot execution and typed data transfer

**Status:** Accepted

**Context:** SciRes is a Julia-first, open-source MATLAB alternative with an IDE written
in Rust. It must support Python, R, and C/C++ for specialized tasks, performance-critical
code, and legacy algorithms without coupling their runtimes or failure modes to the IDE.
The project workspace must coordinate work across independent kernels without implying a
shared interpreter namespace or heap.

**Decision:**

1. The IDE shell and application orchestration are Rust. Its UI uses a native Rust toolkit;
   egui is recommended for the first shell spike, while final toolkit selection remains
   open (ADR-0007).
2. Julia, Python, R, and C/C++ kernels run as separate long-lived processes. Compiled
   C/C++ tasks use isolated workers. The IDE does not load arbitrary native libraries
   into its own process.
3. There is one project workspace, but each kernel owns its own runtime state and variable
   namespace. Values cross kernel boundaries only through explicit, typed transfer
   requests; there is no implicit shared variable namespace.
4. Apache Arrow IPC is the preferred initial representation for supported scientific
   values. Initial transport copies data. Shared-memory transport is deferred until
   benchmarks justify it and buffer ownership, lifetime, cleanup, and failure recovery
   are specified.
5. Initial transferable values include dense N-dimensional fixed-width numeric and
   Boolean arrays with shape/type metadata; UTF-8 strings; rectangular tables with
   supported Arrow primitive columns; and complex values represented by real/imaginary
   pairs. Sparse matrices use compressed sparse column (CSC) representation with Int64
   offsets and indices and zero-based wire indices. Arrow CSC columns use LargeList with
   64-bit list offsets. Language adapters convert between zero-based wire indices and
   each runtime's native index convention. Do not silently densify sparse data.
6. Values without a defined mapping—including custom/runtime objects and ragged arrays—
   remain kernel-local unless the user explicitly converts them. If conversion is not
   supplied, fail the transfer with a clear unsupported-value error; do not silently
   coerce values or substitute a `Value` preview.
7. Kernels implement a versioned SciRes kernel API, using Jupyter-compatible messaging
   where it fits. ADR-0009 and [ADR-0010](0010-direct-rust-julia-transport.md) establish
   and implement direct Rust Jupyter/ZeroMQ transport plus a dedicated loopback Arrow IPC
   data plane (`scires.value` v1) for the Julia adapter (`crates/scires-julia-kernel`),
   wired into `scires-app` via `Command::TransferValue`. Small control messages use JSON
   or Protobuf. The API version and negotiated capabilities, rather than a particular
   transport, define compatibility.

**Consequences:** Kernel crashes and native task failures are isolated from the IDE.
Cross-language exchange has an explicit data-copy cost and must validate type, shape and
capability compatibility. The existing Python-only JSON/NPY/Arrow benchmark is
exploratory format evidence only; it does not verify SciRes adapters or Julia
interoperability.

**V1 implementation:** `crates/scires-interchange` implements Arrow IPC scalar, table,
dense tensor and sparse CSC values. Scalar support is null, Boolean, signed/unsigned
8/16/32/64-bit integer, Float32/64, UTF-8 and component-pair complex values. Tables
preserve Arrow validity bitmaps and support standard primitive, binary, temporal, decimal,
list and struct fields; dictionary,
map, union and extension fields are rejected. A valid NaN is distinct from a missing
value. Dense tensors support Boolean, signed/unsigned 8/16/32/64-bit, Float32/64 and
Complex64/128 values, with contiguous column-major ordering, shape/dtype and optional axis
names/units. Tensor elements cannot be null. Complex values use explicit real/imaginary
components. Sparse matrices use CSC with Int64 offsets/indices, zero-based indices on the wire, and
numeric values. `crates/scires-julia-kernel` converts these indices to Julia's native
one-based `SparseMatrixCSC` indexing over its live loopback Arrow IPC data plane (and the
offline fixture in `tests/julia_arrow_interop.py` also exercises this conversion). The
codec does not coerce dtypes or implement lossy conversion.

Rust unit/doc tests validate round-trips and malformed/unsupported values.
`tests/arrow_profile_interop.py` exercises Rust/PyArrow IPC in both directions, and
`tests/julia_arrow_interop.py` exercises offline file-based Rust↔Julia Arrow IPC.
Separately, `crates/scires-julia-kernel` now implements direct Rust Jupyter/ZeroMQ
control-plane transport (with concurrent `KernelControl::interrupt`, restart, and
shutdown) and live-kernel loopback Arrow IPC data-plane exchange (`scires.value` v1),
live-verified on Julia 1.12.5 + IJulia 1.34.4 + Arrow 2.8.1 across scalars (including
null, NaN, UTF-8, and Complex128), tables (preserving NaN vs. `missing`), column-major
Int32/Complex128 tensors, Float64 CSC sparse matrices, unsupported-value rejection, and
`restart` re-initialization. `crates/scires-app` wires explicit transfers via
`Command::TransferValue`. `tests/julia_kernel_lifecycle.py` remains an external IJulia
conformance fixture only.

Remaining unimplemented areas include variable snapshots, the UI workbench shell and
editor selection (Phase 3), DAP/visualization integration, project/runtime environment
policy, plugin loading/trust/sandboxing, any explicit lossy conversion UX, and
Python/R/C++ kernel adapters (Phase 5). ADR-0009 and ADR-0010 record the Julia adapter
boundary and implemented Rust transport/data plane.
