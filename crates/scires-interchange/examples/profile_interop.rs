//! Manual Rust/PyArrow interoperability fixture for the SciRes Arrow profile.
//!
//! The companion script in `tests/arrow_profile_interop.py` launches this example
//! in both directions and checks the exchanged IPC streams.

use std::env;
use std::error::Error;
use std::fs;

use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use arrow_schema::DataType;
use scires_interchange::{decode, encode, AxisMetadata, DenseTensor, InterchangeValue, TensorData};

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
        "read" => {
            let value = decode(&fs::read(path)?)?;
            let InterchangeValue::Table(batch) = value else {
                return Err("expected a table".into());
            };
            check_python_table(&batch)?;
        }
        _ => return Err("expected `write` or `read`".into()),
    }

    Ok(())
}

fn check_python_table(batch: &RecordBatch) -> Result<(), Box<dyn Error>> {
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
        return Err("Python table values did not round-trip losslessly".into());
    }
    Ok(())
}
