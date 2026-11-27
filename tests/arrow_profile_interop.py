"""Exercise Rust/PyArrow interoperability for the SciRes Arrow value profile v1.

Run with: python tests/arrow_profile_interop.py
Requires the Rust toolchain and the pinned NumPy/PyArrow test dependencies.
"""

import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import pyarrow as pa


ROOT = Path(__file__).resolve().parents[1]
PROFILE_KEY = b"scires.value.envelope"
PROFILE_ID = "scires.value"
PROFILE_VERSION = 1


def cargo_executable():
    candidate = Path.home() / ".cargo" / "bin" / "cargo.exe"
    return str(candidate) if candidate.exists() else (shutil.which("cargo") or "cargo")


def run_example(mode, path):
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


def check_rust_stream(path):
    with pa.ipc.open_stream(path) as reader:
        schema = reader.schema
        envelope = json.loads(schema.metadata[PROFILE_KEY])
        assert envelope == {
            "profile": PROFILE_ID,
            "version": PROFILE_VERSION,
            "value": {
                "kind": "tensor",
                "dtype": "int32",
                "shape": [2, 3],
                "layout": "column_major",
                "axes": [
                    {"name": "row", "unit": "m"},
                    {"name": "column", "unit": None},
                ],
            },
        }
        assert schema.field("values").type == pa.int32()
        assert schema.field("values").nullable is False
        assert reader.read_all().column("values").to_pylist() == [1, 2, 3, 4, 5, 6]


def write_python_table(path):
    envelope = {
        "profile": PROFILE_ID,
        "version": PROFILE_VERSION,
        "value": {"kind": "table"},
    }
    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("value", pa.float64(), nullable=True),
        ],
        metadata={PROFILE_KEY: json.dumps(envelope, separators=(",", ":")).encode()},
    )
    table = pa.Table.from_arrays(
        [
            pa.array([1, 2], type=pa.int64()),
            pa.array([float("nan"), None], type=pa.float64()),
        ],
        schema=schema,
    )
    with pa.OSFile(str(path), "wb") as sink:
        with pa.ipc.new_stream(sink, schema) as writer:
            writer.write_table(table)


def main():
    with tempfile.TemporaryDirectory(prefix="scires-arrow-profile-") as directory:
        rust_stream = Path(directory) / "rust.arrow"
        python_stream = Path(directory) / "python.arrow"

        run_example("write", rust_stream)
        check_rust_stream(rust_stream)

        write_python_table(python_stream)
        run_example("read", python_stream)

    print("Rust/PyArrow Arrow IPC profile v1 interoperability passed")


if __name__ == "__main__":
    main()
