# SciRes Julia kernel adapter

This crate contains the first Julia kernel adapter, outside the toolkit-agnostic
`scires-kernel` port. The minimal reference adapter supervises `julia_bridge.py`, which
uses Python's `jupyter_client` package to launch and control IJulia in a separate process.

The bridge is a vertical-slice implementation, not a production transport. Its
synchronous control path supports startup, code execution, output/error normalization,
restart, and shutdown. It does not yet support interrupt from Rust, variable snapshots,
or transferring Arrow payloads to a live kernel.

For the Julia conformance environment, instantiate `tests/julia/Project.toml` and install
the Python dependencies listed in `tests/requirements-julia-kernel.txt`. Set
`SCIRES_JULIA_PYTHON` to the Python executable containing those dependencies when running
the Rust integration test. Run it explicitly with:

```powershell
$env:SCIRES_JULIA_PYTHON = "path\to\python.exe"
cargo test -p scires-julia-kernel -- --ignored
```
