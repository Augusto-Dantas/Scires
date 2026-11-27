"""Verify Rust <-> Julia interoperability for the SciRes Arrow profile v1.

Run with: python tests/julia_arrow_interop.py
Requires the Julia project in tests/julia and the Rust toolchain.
"""

from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
JULIA_PROJECT = ROOT / "tests" / "julia"
JULIA_SCRIPT = JULIA_PROJECT / "arrow_interop.jl"


def cargo_executable():
    candidate = Path.home() / ".cargo" / "bin" / "cargo.exe"
    return str(candidate) if candidate.exists() else (shutil.which("cargo") or "cargo")


def run_rust(mode, path):
    subprocess.run(
        [
            cargo_executable(),
            "run",
            "--quiet",
            "-p",
            "scires-interchange",
            "--example",
            "profile_interop",
            "--",
            mode,
            str(path),
        ],
        cwd=ROOT,
        check=True,
    )


def run_julia(mode, path):
    julia = shutil.which("julia")
    if julia is None:
        raise RuntimeError("Julia executable was not found on PATH")
    subprocess.run(
        [
            julia,
            "--project=" + str(JULIA_PROJECT),
            str(JULIA_SCRIPT),
            mode,
            str(path),
        ],
        cwd=ROOT,
        check=True,
    )


def main():
    with tempfile.TemporaryDirectory(prefix="scires-julia-arrow-") as directory:
        rust_tensor = Path(directory) / "rust-tensor.arrow"
        rust_sparse = Path(directory) / "rust-sparse.arrow"
        rust_table = Path(directory) / "rust-table.arrow"
        rust_scalar = Path(directory) / "rust-scalar.arrow"
        julia_tensor = Path(directory) / "julia-tensor.arrow"
        julia_sparse = Path(directory) / "julia-sparse.arrow"
        julia_table = Path(directory) / "julia-table.arrow"
        julia_scalar = Path(directory) / "julia-scalar.arrow"

        run_rust("write", rust_tensor)
        run_rust("write_sparse", rust_sparse)
        run_rust("write_table", rust_table)
        run_rust("write_scalar", rust_scalar)
        run_julia("read_tensor", rust_tensor)
        run_julia("read_sparse", rust_sparse)
        run_julia("read_table", rust_table)
        run_julia("read_scalar", rust_scalar)
        run_julia("write_tensor", julia_tensor)
        run_julia("write_sparse", julia_sparse)
        run_julia("write_table", julia_table)
        run_julia("write_scalar", julia_scalar)
        run_rust("read_tensor", julia_tensor)
        run_rust("read_sparse", julia_sparse)
        run_rust("read", julia_table)
        run_rust("read_scalar", julia_scalar)

    print("Rust/Julia Arrow scalar, table, tensor, and zero-based CSC interoperability passed")


if __name__ == "__main__":
    main()
