//! Manual Rust/PyArrow interoperability fixture for the SciRes Arrow profile.
//!
//! The companion script in `tests/arrow_profile_interop.py` launches this example
//! in both directions and checks the exchanged IPC streams.

use std::env;
use std::error::Error;
use std::fs;

use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use scires_interchange::{
    decode, encode, AxisMetadata, CscMatrix, DenseTensor, InterchangeValue, ScalarValue, TensorData,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let mode = args.next().ok_or("expected `write` or `read`")?;
    let path = args.next().ok_or("expected an Arrow IPC path")?;
    let path = std::path::Path::new(&path);

    match mode.as_str() {
        "write" => {
            let tensor = DenseTensor {
                shape: vec![2, 3],
                axes: vec![
                    AxisMetadata {
                        name: Some("row".into()),
                        unit: Some("m".into()),
                    },
                    AxisMetadata {
                        name: Some("column".into()),
                        unit: None,
                    },
                ],
                data: TensorData::Int32(vec![1, 2, 3, 4, 5, 6]),
            };
            fs::write(path, encode(&InterchangeValue::Tensor(tensor))?)?;
        }
        "write_sparse" => {
            let matrix = CscMatrix {
                shape: [3, 2],
                indptr: vec![0, 2, 3],
                indices: vec![0, 2, 1],
                values: TensorData::Float64(vec![10.0, 20.0, 30.0]),
            };
            fs::write(path, encode(&InterchangeValue::SparseCsc(matrix))?)?;
        }
        "write_table" => {
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("value", DataType::Float64, true),
            ]));
            let batch = RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2])),
                    Arc::new(Float64Array::from(vec![Some(f64::NAN), None])),
                ],
            )?;
            fs::write(path, encode(&InterchangeValue::Table(batch))?)?;
        }
        "write_scalar" => {
            fs::write(
                path,
                encode(&InterchangeValue::Scalar(ScalarValue::Int32(Some(42))))?,
            )?;
        }
        "read" => {
            let value = decode(&fs::read(path)?)?;
            let InterchangeValue::Table(batch) = value else {
                return Err("expected a table".into());
            };
            check_table(&batch)?;
        }
        "read_tensor" => {
            let value = decode(&fs::read(path)?)?;
            let InterchangeValue::Tensor(tensor) = value else {
                return Err("expected a tensor".into());
            };
            if tensor.shape != [2, 3]
                || tensor.axes[0].name.as_deref() != Some("row")
                || tensor.axes[0].unit.as_deref() != Some("m")
                || tensor.axes[1].name.as_deref() != Some("column")
                || tensor.axes[1].unit.is_some()
                || tensor.data != TensorData::Int32(vec![1, 2, 3, 4, 5, 6])
            {
                return Err("Julia tensor did not round-trip losslessly".into());
            }
        }
        "read_sparse" => {
            let value = decode(&fs::read(path)?)?;
            let InterchangeValue::SparseCsc(matrix) = value else {
                return Err("expected a sparse matrix".into());
            };
            if matrix.shape != [3, 2]
                || matrix.indptr != [0, 2, 3]
                || matrix.indices != [0, 2, 1]
                || matrix.values != TensorData::Float64(vec![10.0, 20.0, 30.0])
            {
                return Err("Julia CSC matrix did not round-trip losslessly".into());
            }
        }
        "read_scalar" => {
            let value = decode(&fs::read(path)?)?;
            if !matches!(
                value,
                InterchangeValue::Scalar(ScalarValue::Int32(Some(42)))
            ) {
                return Err("Julia scalar did not round-trip losslessly".into());
            }
        }
        _ => return Err("unsupported interoperability fixture mode".into()),
    }

    Ok(())
}

fn check_table(batch: &RecordBatch) -> Result<(), Box<dyn Error>> {
    if batch.num_rows() != 2 || batch.num_columns() != 2 {
        return Err("unexpected table dimensions".into());
    }
    if batch.schema().field(0).data_type() != &DataType::Int64
        || batch.schema().field(1).data_type() != &DataType::Float64
    {
        return Err("unexpected table field types".into());
    }
    let ids = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or("id column is not Int64")?;
    let values = batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or("value column is not Float64")?;
    if ids.values() != &[1, 2]
        || !values.is_valid(0)
        || !values.value(0).is_nan()
        || !values.is_null(1)
    {
        return Err("table values did not round-trip losslessly".into());
    }
    Ok(())
}
