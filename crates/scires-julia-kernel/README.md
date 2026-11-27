# SciRes Julia kernel adapter

This crate implements the Julia kernel adapter outside the pure `scires-kernel` port,
following ADR-0010.

`JuliaKernel` launches an out-of-process IJulia kernel directly via `julia` using an
ephemeral loopback Jupyter connection file (`127.0.0.1`, HMAC-SHA256) and communicates
over pure-Rust ZeroMQ (`zeromq` + `tokio`) with **no Python runtime dependency**.

Supported operations:
- Direct `kernel_info_request` handshake and SciRes Kernel API v1 capability advertisement
- Correlated `execute_request` (`RequestId` ↔ Jupyter `msg_id`), preserving stdout/stderr,
  full MIME display bundles, and structured `ename`/`evalue`/`traceback` execution errors
- Concurrent out-of-band interruption via `KernelControl::interrupt()` over the Jupyter
  `control` channel (`interrupt_request`) with process termination escalation on timeout
- Graceful `shutdown`, fresh-process `restart`, and unexpected exit detection (`Failed`)
- Dedicated local loopback binary TCP Arrow IPC data plane (`scires.value` v1),
  negotiated automatically during handshake when `Arrow.jl` is available in the Julia
  project (leaving `value_profiles: []` if `Arrow.jl` is absent)

## Running the live IJulia and Arrow data-plane integration tests

Instantiate a Julia project containing `IJulia` and `Arrow` (for example `tests/julia`
or a temporary copy) and run:

```sh
SCIRES_JULIA_PROJECT=/path/to/project cargo test -p scires-julia-kernel -- --ignored --nocapture
```

Set `SCIRES_JULIA_EXE` if `julia` is not on `PATH` (Linux also checks `/snap/bin/julia`).
