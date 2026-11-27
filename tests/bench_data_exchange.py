"""Correctness tests and reproducible format benchmarks for numeric array exchange.

This compares Python JSON, NumPy NPY, and Apache Arrow IPC stream encodings.
It exercises format libraries only; it does not claim SciRes kernel integration
or cross-language interoperability.

Run with: python tests/bench_data_exchange.py
"""

import io
import json
import platform
import statistics
import sys
import time
import unittest

import numpy as np
import pyarrow as pa


def json_encode(array):
    payload = {
        "dtype": array.dtype.name,
        "shape": list(array.shape),
        "data": array.tolist(),
    }
    return json.dumps(payload, separators=(",", ":"), allow_nan=False).encode("utf-8")


def json_decode(encoded):
    payload = json.loads(encoded)
    array = np.asarray(payload["data"], dtype=payload["dtype"])
    return array.reshape(payload["shape"])


def npy_encode(array):
    stream = io.BytesIO()
    np.save(stream, array, allow_pickle=False)
    return stream.getvalue()


def npy_decode(encoded):
    return np.load(io.BytesIO(encoded), allow_pickle=False)


def arrow_encode(array):
    flat = pa.array(array.reshape(-1), type=pa.from_numpy_dtype(array.dtype))
    schema = pa.schema(
        [pa.field("value", flat.type)],
        metadata={b"shape": json.dumps(array.shape).encode("ascii")},
    )
    table = pa.Table.from_arrays([flat], schema=schema)
    stream = io.BytesIO()
    with pa.ipc.new_stream(stream, schema) as writer:
        writer.write_table(table)
    return stream.getvalue()


def arrow_decode(encoded):
    table = pa.ipc.open_stream(encoded).read_all()
    shape = tuple(json.loads(table.schema.metadata[b"shape"]))
    return table.column("value").to_numpy().reshape(shape)


FORMATS = (
    ("JSON", json_encode, json_decode),
    ("NPY", npy_encode, npy_decode),
    ("Arrow IPC", arrow_encode, arrow_decode),
)


class ExchangeFormatTests(unittest.TestCase):
    def setUp(self):
        self.array = np.linspace(-1, 1, 128, dtype=np.float64).reshape(8, 16)

    def assert_array_roundtrip(self, encode, decode):
        restored = decode(encode(self.array))
        self.assertEqual(restored.dtype, self.array.dtype)
        self.assertEqual(restored.shape, self.array.shape)
        np.testing.assert_array_equal(restored, self.array)

    def test_numeric_array_roundtrips(self):
        for name, encode, decode in FORMATS:
            with self.subTest(format=name):
                self.assert_array_roundtrip(encode, decode)

    def test_json_preserves_nullable_mixed_records(self):
        records = [
            {"id": 1, "label": "sample", "active": True, "value": None},
            {"id": 2, "label": "control", "active": False, "value": 2.5},
        ]
        restored = json.loads(json.dumps(records))
        self.assertEqual(restored, records)

    def test_arrow_preserves_nullable_mixed_columns(self):
        table = pa.table(
            {
                "id": pa.array([1, 2], type=pa.int64()),
                "label": pa.array(["sample", "control"], type=pa.string()),
                "active": pa.array([True, False], type=pa.bool_()),
                "value": pa.array([None, 2.5], type=pa.float64()),
            }
        )
        stream = io.BytesIO()
        with pa.ipc.new_stream(stream, table.schema) as writer:
            writer.write_table(table)
        restored = pa.ipc.open_stream(stream.getvalue()).read_all()
        self.assertTrue(restored.equals(table))

    def test_npy_rejects_object_arrays_without_pickle(self):
        values = np.array([1, None], dtype=object)
        with self.assertRaises(ValueError):
            npy_encode(values)


def median_milliseconds(samples):
    return statistics.median(samples) * 1_000


def benchmark_one(name, encode, decode, array, warmups, repetitions):
    for _ in range(warmups):
        decoded = decode(encode(array))
        np.testing.assert_array_equal(decoded, array)

    encoded = encode(array)
    encode_samples = []
    decode_samples = []
    for _ in range(repetitions):
        start = time.perf_counter()
        encoded = encode(array)
        encode_samples.append(time.perf_counter() - start)

        start = time.perf_counter()
        decoded = decode(encoded)
        decode_samples.append(time.perf_counter() - start)
        np.testing.assert_array_equal(decoded, array)

    print(
        "{:<10} {:>12} {:>12.3f} {:>12.3f}".format(
            name,
            len(encoded),
            median_milliseconds(encode_samples),
            median_milliseconds(decode_samples),
        )
    )


def run_benchmarks(warmups=3, repetitions=20):
    print(
        "Environment: Python {}; {}; NumPy {}; PyArrow {}".format(
            sys.version.split()[0], platform.platform(), np.__version__, pa.__version__
        )
    )
    print("Warmups: {}; repetitions: {}; statistic: median".format(warmups, repetitions))
    print("Payload: row-major float64 array, values linearly spaced in [-1, 1]")
    print("{:<10} {:>12} {:>12} {:>12}".format("Format", "bytes", "encode ms", "decode ms"))
    for count in (1_000, 100_000):
        array = np.linspace(-1, 1, count, dtype=np.float64).reshape(1, count)
        print("\nElements: {}; raw bytes: {}".format(count, array.nbytes))
        for name, encode, decode in FORMATS:
            benchmark_one(name, encode, decode, array, warmups, repetitions)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ExchangeFormatTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful():
        sys.exit(1)
    print()
    run_benchmarks()
