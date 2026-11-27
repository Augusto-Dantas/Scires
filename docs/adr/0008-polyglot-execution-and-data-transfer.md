# ADR-0008: Polyglot execution and typed data transfer

**Status:** Accepted

**Context:** SciRes is a Julia-first, open-source MATLAB alternative with an IDE written
in Rust. It must support Python, R, and C/C++ for specialized tasks, performance-critical
code, and legacy algorithms without coupling their runtimes or failure modes to the IDE.
The project workspace must coordinate work across independent kernels without implying a
shared interpreter namespace or heap.

**Decision:**

1. The IDE shell and application orchestration are Rust. Its UI uses a native Rust toolkit;
   the choice between egui, Slint, or another native toolkit remains open (ADR-0007).
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
   pairs. Sparse values are transferable only in an explicit typed COO representation
   carrying indices, values, and shape. Do not silently densify sparse data.
6. Values without a defined mapping—including custom/runtime objects and ragged arrays—
   remain kernel-local unless the user explicitly converts them. If conversion is not
   supplied, fail the transfer with a clear unsupported-value error; do not silently
   coerce values or substitute a `Value` preview.
7. Kernels implement a versioned SciRes kernel API, using Jupyter-compatible messaging
   where it fits. Small control messages use JSON or Protobuf. The API version and
   negotiated capabilities, rather than a particular transport, define compatibility.

**Consequences:** Kernel crashes and native task failures are isolated from the IDE.
Cross-language exchange has an explicit data-copy cost and must validate type, shape and
capability compatibility. The existing Python-only JSON/NPY/Arrow benchmark is
exploratory format evidence only; it does not verify SciRes adapters or Julia
interoperability.

Detailed wire schemas, complete numeric type coverage, nullability and memory-order
semantics, sparse index conventions, conversion UX, control-message encoding, transport
details, and the native UI toolkit remain open decisions.
