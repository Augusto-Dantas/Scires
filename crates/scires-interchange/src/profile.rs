use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, LargeListArray, NullArray, RecordBatch, StringArray, StructArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Fields, Schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable profile identifier stored in Arrow schema metadata.
pub const PROFILE_ID: &str = "scires.value";
/// Metadata key containing the serialized profile envelope.
pub const PROFILE_METADATA_KEY: &str = "scires.value.envelope";
/// Version of the SciRes Arrow value profile.
pub const PROFILE_VERSION: u16 = 1;

const INDEX_BASE: u8 = 0;
const COLUMN_MAJOR: &str = "column_major";

/// Errors returned while validating or exchanging a v1 value.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// Arrow IPC encoding, decoding, or schema construction failed.
    #[error("Arrow IPC error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    /// The profile envelope could not be parsed.
    #[error("invalid SciRes value envelope: {0}")]
    Envelope(#[from] serde_json::Error),
    /// The Arrow schema does not contain a SciRes profile envelope.
    #[error("Arrow schema is missing the SciRes value envelope")]
    MissingEnvelope,
    /// The envelope names a different profile.
    #[error("unsupported SciRes value profile `{0}`")]
    UnsupportedProfile(String),
    /// The envelope uses a profile version not supported by this implementation.
    #[error("unsupported SciRes value profile version {0}")]
    UnsupportedVersion(u16),
    /// A valid Arrow type is outside the supported v1 profile.
    #[error("Arrow data type is not supported by the SciRes v1 profile: {0:?}")]
    UnsupportedType(DataType),
    /// A scalar or table violates the v1 shape or type contract.
    #[error("invalid scalar or table value: {0}")]
    InvalidValue(String),
    /// A dense tensor violates the v1 shape, dtype, or layout contract.
    #[error("invalid dense tensor: {0}")]
    InvalidTensor(String),
    /// A sparse CSC matrix violates the v1 index or shape contract.
    #[error("invalid CSC matrix: {0}")]
    InvalidSparse(String),
    /// The IPC stream contains no record batch or more than one batch.
    #[error("v1 values must contain exactly one Arrow record batch")]
    InvalidBatchCount,
}

/// One optional axis label and unit for a dense tensor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisMetadata {
    /// Human-readable axis name.
    pub name: Option<String>,
    /// Unit associated with the axis values.
    pub unit: Option<String>,
}

/// Fixed-width numeric or Boolean tensor element type supported in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorDType {
    /// Boolean values.
    Bool,
    /// Signed 8-bit integers.
    Int8,
    /// Signed 16-bit integers.
    Int16,
    /// Signed 32-bit integers.
    Int32,
    /// Signed 64-bit integers.
    Int64,
    /// Unsigned 8-bit integers.
    UInt8,
    /// Unsigned 16-bit integers.
    UInt16,
    /// Unsigned 32-bit integers.
    UInt32,
    /// Unsigned 64-bit integers.
    UInt64,
    /// 32-bit floating-point values.
    Float32,
    /// 64-bit floating-point values.
    Float64,
    /// Complex values with 32-bit floating-point components.
    Complex64,
    /// Complex values with 64-bit floating-point components.
    Complex128,
}

impl TensorDType {
    fn is_numeric(self) -> bool {
        !matches!(self, Self::Bool)
    }

    fn arrow_type(self) -> DataType {
        match self {
            Self::Bool => DataType::Boolean,
            Self::Int8 => DataType::Int8,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::UInt8 => DataType::UInt8,
            Self::UInt16 => DataType::UInt16,
            Self::UInt32 => DataType::UInt32,
            Self::UInt64 => DataType::UInt64,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Complex64 => complex_type(DataType::Float32),
            Self::Complex128 => complex_type(DataType::Float64),
        }
    }
}

/// Complex number represented by real and imaginary components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex32 {
    /// Real component.
    pub real: f32,
    /// Imaginary component.
    pub imaginary: f32,
}

/// Complex number represented by real and imaginary components.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex64 {
    /// Real component.
    pub real: f64,
    /// Imaginary component.
    pub imaginary: f64,
}

/// Owned, column-major tensor payload with no implicit numeric conversion.
#[derive(Debug, Clone, PartialEq)]
pub enum TensorData {
    /// Boolean elements.
    Bool(Vec<bool>),
    /// Signed 8-bit integer elements.
    Int8(Vec<i8>),
    /// Signed 16-bit integer elements.
    Int16(Vec<i16>),
    /// Signed 32-bit integer elements.
    Int32(Vec<i32>),
    /// Signed 64-bit integer elements.
    Int64(Vec<i64>),
    /// Unsigned 8-bit integer elements.
    UInt8(Vec<u8>),
    /// Unsigned 16-bit integer elements.
    UInt16(Vec<u16>),
    /// Unsigned 32-bit integer elements.
    UInt32(Vec<u32>),
    /// Unsigned 64-bit integer elements.
    UInt64(Vec<u64>),
    /// 32-bit floating-point elements.
    Float32(Vec<f32>),
    /// 64-bit floating-point elements.
    Float64(Vec<f64>),
    /// Complex elements with 32-bit components.
    Complex64(Vec<Complex32>),
    /// Complex elements with 64-bit components.
    Complex128(Vec<Complex64>),
}

impl TensorData {
    /// Returns the element type of this payload.
    pub fn dtype(&self) -> TensorDType {
        match self {
            Self::Bool(_) => TensorDType::Bool,
            Self::Int8(_) => TensorDType::Int8,
            Self::Int16(_) => TensorDType::Int16,
            Self::Int32(_) => TensorDType::Int32,
            Self::Int64(_) => TensorDType::Int64,
            Self::UInt8(_) => TensorDType::UInt8,
            Self::UInt16(_) => TensorDType::UInt16,
            Self::UInt32(_) => TensorDType::UInt32,
            Self::UInt64(_) => TensorDType::UInt64,
            Self::Float32(_) => TensorDType::Float32,
            Self::Float64(_) => TensorDType::Float64,
            Self::Complex64(_) => TensorDType::Complex64,
            Self::Complex128(_) => TensorDType::Complex128,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Bool(values) => values.len(),
            Self::Int8(values) => values.len(),
            Self::Int16(values) => values.len(),
            Self::Int32(values) => values.len(),
            Self::Int64(values) => values.len(),
            Self::UInt8(values) => values.len(),
            Self::UInt16(values) => values.len(),
            Self::UInt32(values) => values.len(),
            Self::UInt64(values) => values.len(),
            Self::Float32(values) => values.len(),
            Self::Float64(values) => values.len(),
            Self::Complex64(values) => values.len(),
            Self::Complex128(values) => values.len(),
        }
    }

    fn to_array(&self) -> ArrayRef {
        match self {
            Self::Bool(values) => Arc::new(BooleanArray::from(values.clone())),
            Self::Int8(values) => Arc::new(Int8Array::from(values.clone())),
            Self::Int16(values) => Arc::new(Int16Array::from(values.clone())),
            Self::Int32(values) => Arc::new(Int32Array::from(values.clone())),
            Self::Int64(values) => Arc::new(Int64Array::from(values.clone())),
            Self::UInt8(values) => Arc::new(UInt8Array::from(values.clone())),
            Self::UInt16(values) => Arc::new(UInt16Array::from(values.clone())),
            Self::UInt32(values) => Arc::new(UInt32Array::from(values.clone())),
            Self::UInt64(values) => Arc::new(UInt64Array::from(values.clone())),
            Self::Float32(values) => Arc::new(Float32Array::from(values.clone())),
            Self::Float64(values) => Arc::new(Float64Array::from(values.clone())),
            Self::Complex64(values) => complex_array_f32(values),
            Self::Complex128(values) => complex_array_f64(values),
        }
    }

    fn from_array(dtype: TensorDType, array: &dyn Array) -> Result<Self, ProfileError> {
        if array.data_type() != &dtype.arrow_type() {
            return Err(ProfileError::InvalidTensor(format!(
                "envelope dtype {dtype:?} does not match Arrow type {:?}",
                array.data_type()
            )));
        }
        if array.null_count() != 0 {
            return Err(ProfileError::InvalidTensor(
                "dense tensor elements cannot be null; use a table for nullable data".into(),
            ));
        }

        macro_rules! primitive_values {
            ($array:expr, $array_type:ty) => {{
                $array
                    .as_any()
                    .downcast_ref::<$array_type>()
                    .ok_or_else(|| ProfileError::InvalidTensor("Arrow array type mismatch".into()))?
                    .values()
                    .to_vec()
            }};
        }

        Ok(match dtype {
            TensorDType::Bool => {
                let values = array
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .ok_or_else(|| ProfileError::InvalidTensor("Arrow array type mismatch".into()))?
                    .iter()
                    .map(|value| {
                        value.ok_or_else(|| {
                            ProfileError::InvalidTensor("unexpected null Boolean".into())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Self::Bool(values)
            }
            TensorDType::Int8 => Self::Int8(primitive_values!(array, Int8Array)),
            TensorDType::Int16 => Self::Int16(primitive_values!(array, Int16Array)),
            TensorDType::Int32 => Self::Int32(primitive_values!(array, Int32Array)),
            TensorDType::Int64 => Self::Int64(primitive_values!(array, Int64Array)),
            TensorDType::UInt8 => Self::UInt8(primitive_values!(array, UInt8Array)),
            TensorDType::UInt16 => Self::UInt16(primitive_values!(array, UInt16Array)),
            TensorDType::UInt32 => Self::UInt32(primitive_values!(array, UInt32Array)),
            TensorDType::UInt64 => Self::UInt64(primitive_values!(array, UInt64Array)),
            TensorDType::Float32 => Self::Float32(primitive_values!(array, Float32Array)),
            TensorDType::Float64 => Self::Float64(primitive_values!(array, Float64Array)),
            TensorDType::Complex64 => {
                let array = array
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("complex tensor must be a struct".into())
                    })?;
                let real = array
                    .column_by_name("real")
                    .and_then(|array| array.as_any().downcast_ref::<Float32Array>())
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("missing Float32 real component".into())
                    })?;
                let imaginary = array
                    .column_by_name("imaginary")
                    .and_then(|array| array.as_any().downcast_ref::<Float32Array>())
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("missing Float32 imaginary component".into())
                    })?;
                if real.null_count() != 0 || imaginary.null_count() != 0 {
                    return Err(ProfileError::InvalidTensor(
                        "complex tensor components cannot be null".into(),
                    ));
                }
                Self::Complex64(
                    real.values()
                        .iter()
                        .zip(imaginary.values())
                        .map(|(real, imaginary)| Complex32 {
                            real: *real,
                            imaginary: *imaginary,
                        })
                        .collect(),
                )
            }
            TensorDType::Complex128 => {
                let array = array
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("complex tensor must be a struct".into())
                    })?;
                let real = array
                    .column_by_name("real")
                    .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("missing Float64 real component".into())
                    })?;
                let imaginary = array
                    .column_by_name("imaginary")
                    .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
                    .ok_or_else(|| {
                        ProfileError::InvalidTensor("missing Float64 imaginary component".into())
                    })?;
                if real.null_count() != 0 || imaginary.null_count() != 0 {
                    return Err(ProfileError::InvalidTensor(
                        "complex tensor components cannot be null".into(),
                    ));
                }
                Self::Complex128(
                    real.values()
                        .iter()
                        .zip(imaginary.values())
                        .map(|(real, imaginary)| Complex64 {
                            real: *real,
                            imaginary: *imaginary,
                        })
                        .collect(),
                )
            }
        })
    }
}

/// Dense N-dimensional numeric or Boolean array in contiguous column-major order.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseTensor {
    /// Tensor dimensions in logical axis order.
    pub shape: Vec<u64>,
    /// Optional per-axis names and units; empty means no axis metadata.
    pub axes: Vec<AxisMetadata>,
    /// Flattened data, with the first dimension varying fastest.
    pub data: TensorData,
}

/// One Arrow-compatible scalar. `None` represents missing, distinct from NaN.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    /// Arrow null scalar.
    Null,
    /// Boolean scalar.
    Boolean(Option<bool>),
    /// Signed 8-bit integer scalar.
    Int8(Option<i8>),
    /// Signed 16-bit integer scalar.
    Int16(Option<i16>),
    /// Signed 32-bit integer scalar.
    Int32(Option<i32>),
    /// Signed 64-bit integer scalar.
    Int64(Option<i64>),
    /// Unsigned 8-bit integer scalar.
    UInt8(Option<u8>),
    /// Unsigned 16-bit integer scalar.
    UInt16(Option<u16>),
    /// Unsigned 32-bit integer scalar.
    UInt32(Option<u32>),
    /// Unsigned 64-bit integer scalar.
    UInt64(Option<u64>),
    /// 32-bit floating-point scalar.
    Float32(Option<f32>),
    /// 64-bit floating-point scalar.
    Float64(Option<f64>),
    /// UTF-8 string scalar.
    Utf8(Option<String>),
    /// Complex scalar with Float32 real and imaginary components.
    Complex64(Option<Complex32>),
    /// Complex scalar with Float64 real and imaginary components.
    Complex128(Option<Complex64>),
}

/// Sparse matrix in compressed sparse column format.
#[derive(Debug, Clone, PartialEq)]
pub struct CscMatrix {
    /// Matrix dimensions `[rows, columns]`.
    pub shape: [u64; 2],
    /// Zero-based column offsets, with length `columns + 1`.
    pub indptr: Vec<i64>,
    /// Zero-based row indices, strictly increasing within each column.
    pub indices: Vec<i64>,
    /// Values associated with row indices.
    pub values: TensorData,
}

/// A v1 value supported by the SciRes Arrow IPC profile.
#[derive(Debug, Clone)]
pub enum InterchangeValue {
    /// Standard Arrow scalar represented by a one-row, one-column batch.
    Scalar(ScalarValue),
    /// Standard Arrow record batch with supported field types.
    Table(RecordBatch),
    /// Dense, fixed-width N-dimensional tensor.
    Tensor(DenseTensor),
    /// Sparse matrix in zero-based CSC form.
    SparseCsc(CscMatrix),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    profile: String,
    version: u16,
    value: ValueMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ValueMetadata {
    Scalar,
    Table,
    Tensor {
        dtype: TensorDType,
        shape: Vec<u64>,
        layout: String,
        axes: Vec<AxisMetadata>,
    },
    SparseCsc {
        dtype: TensorDType,
        shape: [u64; 2],
        index_base: u8,
    },
}

/// Encodes one value as an Arrow IPC stream with a versioned SciRes envelope.
///
/// The function performs no dtype coercion. Unsupported Arrow table types and
/// malformed tensor or sparse metadata return errors.
///
/// # Errors
///
/// Returns [`ProfileError`] if the value violates the profile or Arrow cannot
/// encode it.
pub fn encode(value: &InterchangeValue) -> Result<Vec<u8>, ProfileError> {
    let (batch, value_metadata) = match value {
        InterchangeValue::Scalar(value) => (scalar_batch(value)?, ValueMetadata::Scalar),
        InterchangeValue::Table(batch) => {
            validate_table(batch)?;
            (batch.clone(), ValueMetadata::Table)
        }
        InterchangeValue::Tensor(tensor) => {
            validate_tensor(tensor)?;
            let array = tensor.data.to_array();
            let field = Field::new("values", array.data_type().clone(), false);
            let schema = Arc::new(Schema::new(vec![field]));
            let batch = RecordBatch::try_new(schema, vec![array])?;
            (
                batch,
                ValueMetadata::Tensor {
                    dtype: tensor.data.dtype(),
                    shape: tensor.shape.clone(),
                    layout: COLUMN_MAJOR.to_owned(),
                    axes: tensor.axes.clone(),
                },
            )
        }
        InterchangeValue::SparseCsc(matrix) => {
            validate_sparse(matrix)?;
            let pointers: ArrayRef = Arc::new(Int64Array::from(matrix.indptr.clone()));
            let indices: ArrayRef = Arc::new(Int64Array::from(matrix.indices.clone()));
            let values = matrix.values.to_array();
            let columns = vec![
                singleton_list(pointers)?,
                singleton_list(indices)?,
                singleton_list(values)?,
            ];
            let fields = ["indptr", "indices", "values"]
                .into_iter()
                .zip(columns.iter())
                .map(|(name, array)| Field::new(name, array.data_type().clone(), false))
                .collect::<Vec<_>>();
            let schema = Arc::new(Schema::new(fields));
            let batch = RecordBatch::try_new(schema, columns)?;
            (
                batch,
                ValueMetadata::SparseCsc {
                    dtype: matrix.values.dtype(),
                    shape: matrix.shape,
                    index_base: INDEX_BASE,
                },
            )
        }
    };

    let envelope = Envelope {
        profile: PROFILE_ID.to_owned(),
        version: PROFILE_VERSION,
        value: value_metadata,
    };
    encode_batch(&batch, envelope)
}

/// Decodes and validates one Arrow IPC stream from the SciRes v1 value profile.
///
/// # Errors
///
/// Returns [`ProfileError`] for malformed metadata, unsupported types, invalid
/// array layouts, or malformed Arrow IPC data.
pub fn decode(bytes: &[u8]) -> Result<InterchangeValue, ProfileError> {
    let mut reader = StreamReader::try_new(Cursor::new(bytes), None)?;
    let schema = reader.schema();
    let envelope = schema
        .metadata()
        .get(PROFILE_METADATA_KEY)
        .ok_or(ProfileError::MissingEnvelope)?;
    let envelope: Envelope = serde_json::from_str(envelope)?;
    if envelope.profile != PROFILE_ID {
        return Err(ProfileError::UnsupportedProfile(envelope.profile));
    }
    if envelope.version != PROFILE_VERSION {
        return Err(ProfileError::UnsupportedVersion(envelope.version));
    }
    let batch = reader
        .next()
        .transpose()?
        .ok_or(ProfileError::InvalidBatchCount)?;
    if reader.next().transpose()?.is_some() {
        return Err(ProfileError::InvalidBatchCount);
    }

    match envelope.value {
        ValueMetadata::Scalar => decode_scalar(&batch).map(InterchangeValue::Scalar),
        ValueMetadata::Table => {
            validate_table(&batch)?;
            Ok(InterchangeValue::Table(strip_envelope(batch)?))
        }
        ValueMetadata::Tensor {
            dtype,
            shape,
            layout,
            axes,
        } => {
            if layout != COLUMN_MAJOR {
                return Err(ProfileError::InvalidTensor(format!(
                    "unsupported tensor layout `{layout}`"
                )));
            }
            validate_tensor_batch(&schema, &batch)?;
            if batch.num_columns() != 1 || batch.schema().field(0).name() != "values" {
                return Err(ProfileError::InvalidTensor(
                    "tensor batch must contain one `values` column".into(),
                ));
            }
            let array = batch.column(0);
            let data = TensorData::from_array(dtype, array.as_ref())?;
            let tensor = DenseTensor { shape, axes, data };
            validate_tensor(&tensor)?;
            Ok(InterchangeValue::Tensor(tensor))
        }
        ValueMetadata::SparseCsc {
            dtype,
            shape,
            index_base,
        } => {
            if index_base != INDEX_BASE {
                return Err(ProfileError::InvalidSparse(format!(
                    "wire index base must be zero, got {index_base}"
                )));
            }
            decode_sparse(&batch, dtype, shape).map(InterchangeValue::SparseCsc)
        }
    }
}

fn encode_batch(batch: &RecordBatch, envelope: Envelope) -> Result<Vec<u8>, ProfileError> {
    let schema = batch.schema();
    let mut metadata = schema.metadata().clone();
    if metadata.contains_key(PROFILE_METADATA_KEY) {
        return Err(ProfileError::InvalidValue(
            "input schema uses the reserved SciRes envelope metadata key".into(),
        ));
    }
    metadata.insert(
        PROFILE_METADATA_KEY.to_owned(),
        serde_json::to_string(&envelope)?,
    );
    let schema = Arc::new(Schema::new_with_metadata(schema.fields().clone(), metadata));
    let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec())?;
    let mut bytes = Vec::new();
    let mut writer = StreamWriter::try_new(&mut bytes, schema.as_ref())?;
    writer.write(&batch)?;
    writer.finish()?;
    drop(writer);
    Ok(bytes)
}

fn scalar_batch(value: &ScalarValue) -> Result<RecordBatch, ProfileError> {
    let array = scalar_array(value);
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        array.data_type().clone(),
        true,
    )]));
    Ok(RecordBatch::try_new(schema, vec![array])?)
}

fn scalar_array(value: &ScalarValue) -> ArrayRef {
    match value {
        ScalarValue::Null => Arc::new(NullArray::new(1)),
        ScalarValue::Boolean(value) => Arc::new(BooleanArray::from(vec![*value])),
        ScalarValue::Int8(value) => Arc::new(Int8Array::from(vec![*value])),
        ScalarValue::Int16(value) => Arc::new(Int16Array::from(vec![*value])),
        ScalarValue::Int32(value) => Arc::new(Int32Array::from(vec![*value])),
        ScalarValue::Int64(value) => Arc::new(Int64Array::from(vec![*value])),
        ScalarValue::UInt8(value) => Arc::new(UInt8Array::from(vec![*value])),
        ScalarValue::UInt16(value) => Arc::new(UInt16Array::from(vec![*value])),
        ScalarValue::UInt32(value) => Arc::new(UInt32Array::from(vec![*value])),
        ScalarValue::UInt64(value) => Arc::new(UInt64Array::from(vec![*value])),
        ScalarValue::Float32(value) => Arc::new(Float32Array::from(vec![*value])),
        ScalarValue::Float64(value) => Arc::new(Float64Array::from(vec![*value])),
        ScalarValue::Utf8(value) => Arc::new(StringArray::from(vec![value.as_deref()])),
        ScalarValue::Complex64(value) => {
            let zero = Complex32 {
                real: 0.0,
                imaginary: 0.0,
            };
            complex_array_f32_with_nulls(
                &[value.unwrap_or(zero)],
                value.is_none().then(|| NullBuffer::new_null(1)),
            )
        }
        ScalarValue::Complex128(value) => {
            let zero = Complex64 {
                real: 0.0,
                imaginary: 0.0,
            };
            complex_array_f64_with_nulls(
                &[value.unwrap_or(zero)],
                value.is_none().then(|| NullBuffer::new_null(1)),
            )
        }
    }
}

fn decode_scalar(batch: &RecordBatch) -> Result<ScalarValue, ProfileError> {
    if batch.num_rows() != 1
        || batch.num_columns() != 1
        || batch.schema().field(0).name() != "value"
    {
        return Err(ProfileError::InvalidValue(
            "scalar batch must contain one `value` column and one row".into(),
        ));
    }
    let array = batch.column(0);
    if array.is_null(0) {
        return Ok(match array.data_type() {
            DataType::Null => ScalarValue::Null,
            DataType::Boolean => ScalarValue::Boolean(None),
            DataType::Int8 => ScalarValue::Int8(None),
            DataType::Int16 => ScalarValue::Int16(None),
            DataType::Int32 => ScalarValue::Int32(None),
            DataType::Int64 => ScalarValue::Int64(None),
            DataType::UInt8 => ScalarValue::UInt8(None),
            DataType::UInt16 => ScalarValue::UInt16(None),
            DataType::UInt32 => ScalarValue::UInt32(None),
            DataType::UInt64 => ScalarValue::UInt64(None),
            DataType::Float32 => ScalarValue::Float32(None),
            DataType::Float64 => ScalarValue::Float64(None),
            DataType::Utf8 => ScalarValue::Utf8(None),
            data_type if data_type == &complex_type(DataType::Float32) => {
                ScalarValue::Complex64(None)
            }
            data_type if data_type == &complex_type(DataType::Float64) => {
                ScalarValue::Complex128(None)
            }
            data_type => return Err(ProfileError::UnsupportedType(data_type.clone())),
        });
    }
    macro_rules! scalar_value {
        ($array:expr, $array_type:ty, $variant:ident) => {{
            let array = $array
                .as_any()
                .downcast_ref::<$array_type>()
                .ok_or_else(|| ProfileError::InvalidValue("Arrow scalar type mismatch".into()))?;
            ScalarValue::$variant(Some(array.value(0)))
        }};
    }
    Ok(match array.data_type() {
        DataType::Null => ScalarValue::Null,
        DataType::Boolean => scalar_value!(array, BooleanArray, Boolean),
        DataType::Int8 => scalar_value!(array, Int8Array, Int8),
        DataType::Int16 => scalar_value!(array, Int16Array, Int16),
        DataType::Int32 => scalar_value!(array, Int32Array, Int32),
        DataType::Int64 => scalar_value!(array, Int64Array, Int64),
        DataType::UInt8 => scalar_value!(array, UInt8Array, UInt8),
        DataType::UInt16 => scalar_value!(array, UInt16Array, UInt16),
        DataType::UInt32 => scalar_value!(array, UInt32Array, UInt32),
        DataType::UInt64 => scalar_value!(array, UInt64Array, UInt64),
        DataType::Float32 => scalar_value!(array, Float32Array, Float32),
        DataType::Float64 => scalar_value!(array, Float64Array, Float64),
        DataType::Utf8 => {
            let array = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| ProfileError::InvalidValue("Arrow scalar type mismatch".into()))?;
            ScalarValue::Utf8(Some(array.value(0).to_owned()))
        }
        data_type if data_type == &complex_type(DataType::Float32) => {
            ScalarValue::Complex64(Some(decode_complex_scalar_f32(array.as_ref())?))
        }
        data_type if data_type == &complex_type(DataType::Float64) => {
            ScalarValue::Complex128(Some(decode_complex_scalar_f64(array.as_ref())?))
        }
        data_type => return Err(ProfileError::UnsupportedType(data_type.clone())),
    })
}

fn validate_table(batch: &RecordBatch) -> Result<(), ProfileError> {
    for field in batch.schema().fields() {
        if field.metadata().contains_key("ARROW:extension:name")
            || !supported_table_type(field.data_type())
        {
            return Err(ProfileError::UnsupportedType(field.data_type().clone()));
        }
    }
    Ok(())
}

fn supported_table_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::Null
        | DataType::Boolean
        | DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Binary
        | DataType::LargeBinary
        | DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Date32
        | DataType::Date64
        | DataType::Timestamp(_, _)
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Duration(_)
        | DataType::Interval(_)
        | DataType::Decimal128(_, _)
        | DataType::Decimal256(_, _)
        | DataType::FixedSizeBinary(_) => true,
        DataType::List(field) | DataType::LargeList(field) => supported_table_field(field.as_ref()),
        DataType::FixedSizeList(field, _) => supported_table_field(field.as_ref()),
        DataType::Struct(fields) => fields
            .iter()
            .all(|field| supported_table_field(field.as_ref())),
        _ => false,
    }
}

fn supported_table_field(field: &Field) -> bool {
    !field.metadata().contains_key("ARROW:extension:name")
        && supported_table_type(field.data_type())
}

fn validate_tensor(tensor: &DenseTensor) -> Result<(), ProfileError> {
    if tensor.shape.is_empty() {
        return Err(ProfileError::InvalidTensor(
            "tensor rank must be at least one; encode scalars as Arrow scalars".into(),
        ));
    }
    if !tensor.axes.is_empty() && tensor.axes.len() != tensor.shape.len() {
        return Err(ProfileError::InvalidTensor(
            "axis metadata count must be zero or equal to tensor rank".into(),
        ));
    }
    let element_count = tensor
        .shape
        .iter()
        .try_fold(1_u64, |count, dimension| count.checked_mul(*dimension))
        .ok_or_else(|| ProfileError::InvalidTensor("shape product overflows u64".into()))?;
    if usize::try_from(element_count).ok() != Some(tensor.data.len()) {
        return Err(ProfileError::InvalidTensor(format!(
            "shape describes {element_count} values but payload contains {}",
            tensor.data.len()
        )));
    }
    Ok(())
}

fn validate_sparse(matrix: &CscMatrix) -> Result<(), ProfileError> {
    if !matrix.values.dtype().is_numeric() {
        return Err(ProfileError::InvalidSparse(
            "CSC matrix values must have a numeric dtype".into(),
        ));
    }
    let columns = usize::try_from(matrix.shape[1])
        .map_err(|_| ProfileError::InvalidSparse("column count exceeds host limits".into()))?;
    let expected_offsets = columns
        .checked_add(1)
        .ok_or_else(|| ProfileError::InvalidSparse("column count overflows".into()))?;
    if matrix.indptr.len() != expected_offsets {
        return Err(ProfileError::InvalidSparse(format!(
            "indptr length must be columns + 1 ({expected_offsets})"
        )));
    }
    if matrix.indptr.first() != Some(&0) {
        return Err(ProfileError::InvalidSparse(
            "zero-based indptr must start at zero".into(),
        ));
    }
    if matrix.indptr.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(ProfileError::InvalidSparse(
            "indptr offsets must be nondecreasing".into(),
        ));
    }
    let nonzeros = i64::try_from(matrix.values.len())
        .map_err(|_| ProfileError::InvalidSparse("nonzero count exceeds i64".into()))?;
    if matrix.indptr.last() != Some(&nonzeros) || matrix.indices.len() != matrix.values.len() {
        return Err(ProfileError::InvalidSparse(
            "last offset, indices, and values must describe the same nonzero count".into(),
        ));
    }
    for column in 0..columns {
        let start = usize::try_from(matrix.indptr[column])
            .map_err(|_| ProfileError::InvalidSparse("negative column offset".into()))?;
        let end = usize::try_from(matrix.indptr[column + 1])
            .map_err(|_| ProfileError::InvalidSparse("negative column offset".into()))?;
        let indices = &matrix.indices[start..end];
        if indices
            .iter()
            .any(|index| *index < 0 || (*index as u64) >= matrix.shape[0])
        {
            return Err(ProfileError::InvalidSparse(
                "row indices must be zero-based and within matrix shape".into(),
            ));
        }
        if indices.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ProfileError::InvalidSparse(
                "row indices must be strictly increasing within each column".into(),
            ));
        }
    }
    Ok(())
}

fn decode_sparse(
    batch: &RecordBatch,
    dtype: TensorDType,
    shape: [u64; 2],
) -> Result<CscMatrix, ProfileError> {
    if batch.num_rows() != 1
        || batch.num_columns() != 3
        || batch.schema().field(0).name() != "indptr"
        || batch.schema().field(1).name() != "indices"
        || batch.schema().field(2).name() != "values"
    {
        return Err(ProfileError::InvalidSparse(
            "CSC batch must have one row with indptr, indices, values lists".into(),
        ));
    }
    let indptr = list_values(batch.column(0))?;
    let indices = list_values(batch.column(1))?;
    let values = list_values(batch.column(2))?;
    if indptr.null_count() != 0 || indices.null_count() != 0 {
        return Err(ProfileError::InvalidSparse(
            "CSC offsets and indices cannot contain nulls".into(),
        ));
    }
    let indptr = indptr
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| ProfileError::InvalidSparse("indptr must contain Int64".into()))?
        .values()
        .to_vec();
    let indices = indices
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| ProfileError::InvalidSparse("indices must contain Int64".into()))?
        .values()
        .to_vec();
    let matrix = CscMatrix {
        shape,
        indptr,
        indices,
        values: TensorData::from_array(dtype, values.as_ref())
            .map_err(|error| ProfileError::InvalidSparse(error.to_string()))?,
    };
    validate_sparse(&matrix)?;
    Ok(matrix)
}

fn list_values(array: &ArrayRef) -> Result<ArrayRef, ProfileError> {
    let list = array
        .as_any()
        .downcast_ref::<LargeListArray>()
        .ok_or_else(|| {
            ProfileError::InvalidSparse("CSC columns must be Arrow large lists".into())
        })?;
    if list.len() != 1 || list.is_null(0) {
        return Err(ProfileError::InvalidSparse(
            "CSC list columns must contain one non-null list".into(),
        ));
    }
    Ok(list.value(0))
}

fn validate_tensor_batch(schema: &Schema, batch: &RecordBatch) -> Result<(), ProfileError> {
    if batch.num_columns() != 1
        || schema.field(0).name() != "values"
        || schema.field(0).is_nullable()
    {
        return Err(ProfileError::InvalidTensor(
            "tensor batch must contain one non-nullable `values` column".into(),
        ));
    }
    Ok(())
}

fn strip_envelope(batch: RecordBatch) -> Result<RecordBatch, ProfileError> {
    let schema = batch.schema();
    let mut metadata = schema.metadata().clone();
    metadata.remove(PROFILE_METADATA_KEY);
    let schema = Arc::new(Schema::new_with_metadata(schema.fields().clone(), metadata));
    Ok(RecordBatch::try_new(schema, batch.columns().to_vec())?)
}

fn complex_type(component_type: DataType) -> DataType {
    DataType::Struct(Fields::from(vec![
        Arc::new(Field::new("real", component_type.clone(), false)),
        Arc::new(Field::new("imaginary", component_type, false)),
    ]))
}

fn complex_array_f32(values: &[Complex32]) -> ArrayRef {
    complex_array_f32_with_nulls(values, None)
}

fn complex_array_f32_with_nulls(values: &[Complex32], nulls: Option<NullBuffer>) -> ArrayRef {
    let real = Arc::new(Float32Array::from(
        values.iter().map(|value| value.real).collect::<Vec<_>>(),
    ));
    let imaginary = Arc::new(Float32Array::from(
        values
            .iter()
            .map(|value| value.imaginary)
            .collect::<Vec<_>>(),
    ));
    Arc::new(StructArray::new(
        match complex_type(DataType::Float32) {
            DataType::Struct(fields) => fields,
            _ => unreachable!(),
        },
        vec![real, imaginary],
        nulls,
    ))
}

fn complex_array_f64(values: &[Complex64]) -> ArrayRef {
    complex_array_f64_with_nulls(values, None)
}

fn complex_array_f64_with_nulls(values: &[Complex64], nulls: Option<NullBuffer>) -> ArrayRef {
    let real = Arc::new(Float64Array::from(
        values.iter().map(|value| value.real).collect::<Vec<_>>(),
    ));
    let imaginary = Arc::new(Float64Array::from(
        values
            .iter()
            .map(|value| value.imaginary)
            .collect::<Vec<_>>(),
    ));
    Arc::new(StructArray::new(
        match complex_type(DataType::Float64) {
            DataType::Struct(fields) => fields,
            _ => unreachable!(),
        },
        vec![real, imaginary],
        nulls,
    ))
}

fn decode_complex_scalar_f32(array: &dyn Array) -> Result<Complex32, ProfileError> {
    let array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| ProfileError::InvalidValue("complex scalar must be a struct".into()))?;
    let real = array
        .column_by_name("real")
        .and_then(|array| array.as_any().downcast_ref::<Float32Array>())
        .ok_or_else(|| ProfileError::InvalidValue("missing Float32 real component".into()))?;
    let imaginary = array
        .column_by_name("imaginary")
        .and_then(|array| array.as_any().downcast_ref::<Float32Array>())
        .ok_or_else(|| ProfileError::InvalidValue("missing Float32 imaginary component".into()))?;
    if real.null_count() != 0 || imaginary.null_count() != 0 {
        return Err(ProfileError::InvalidValue(
            "complex scalar components cannot be null".into(),
        ));
    }
    Ok(Complex32 {
        real: real.value(0),
        imaginary: imaginary.value(0),
    })
}

fn decode_complex_scalar_f64(array: &dyn Array) -> Result<Complex64, ProfileError> {
    let array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| ProfileError::InvalidValue("complex scalar must be a struct".into()))?;
    let real = array
        .column_by_name("real")
        .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
        .ok_or_else(|| ProfileError::InvalidValue("missing Float64 real component".into()))?;
    let imaginary = array
        .column_by_name("imaginary")
        .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
        .ok_or_else(|| ProfileError::InvalidValue("missing Float64 imaginary component".into()))?;
    if real.null_count() != 0 || imaginary.null_count() != 0 {
        return Err(ProfileError::InvalidValue(
            "complex scalar components cannot be null".into(),
        ));
    }
    Ok(Complex64 {
        real: real.value(0),
        imaginary: imaginary.value(0),
    })
}

fn singleton_list(values: ArrayRef) -> Result<ArrayRef, ProfileError> {
    let length = i64::try_from(values.len())
        .map_err(|_| ProfileError::InvalidSparse("CSC list length exceeds i64".into()))?;
    let offsets = OffsetBuffer::new(ScalarBuffer::from(vec![0_i64, length]));
    let field = Arc::new(Field::new("item", values.data_type().clone(), true));
    Ok(Arc::new(LargeListArray::new(field, offsets, values, None)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{new_empty_array, Float64Array, Int64Array};
    use arrow_schema::Schema;

    fn table_with_missing_and_nan() -> RecordBatch {
        let floats = Arc::new(Float64Array::from(vec![Some(f64::NAN), None, Some(3.5)]));
        let names = Arc::new(StringArray::from(vec![
            Some("nan"),
            Some("missing"),
            Some("value"),
        ]));
        let schema = Arc::new(Schema::new(vec![
            Field::new("value", DataType::Float64, true),
            Field::new("name", DataType::Utf8, true),
        ]));
        RecordBatch::try_new(schema, vec![floats, names]).expect("test batch is valid")
    }

    fn rewrite_envelope(bytes: &[u8], update: impl FnOnce(&mut Envelope)) -> Vec<u8> {
        let mut reader =
            StreamReader::try_new(Cursor::new(bytes), None).expect("encoded stream is valid");
        let schema = reader.schema();
        let mut envelope: Envelope = serde_json::from_str(
            schema
                .metadata()
                .get(PROFILE_METADATA_KEY)
                .expect("test stream has envelope"),
        )
        .expect("test envelope is valid");
        update(&mut envelope);
        let batch = reader
            .next()
            .expect("encoded stream contains a batch")
            .expect("test batch is valid");
        let mut metadata = schema.metadata().clone();
        metadata.insert(
            PROFILE_METADATA_KEY.to_owned(),
            serde_json::to_string(&envelope).expect("envelope serializes"),
        );
        let schema = Arc::new(Schema::new_with_metadata(schema.fields().clone(), metadata));
        let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec())
            .expect("metadata rewrite preserves schema");
        let mut output = Vec::new();
        let mut writer =
            StreamWriter::try_new(&mut output, schema.as_ref()).expect("writer is valid");
        writer.write(&batch).expect("batch writes");
        writer.finish().expect("stream finishes");
        drop(writer);
        output
    }

    #[test]
    fn table_roundtrip_preserves_null_separately_from_nan() {
        let encoded =
            encode(&InterchangeValue::Table(table_with_missing_and_nan())).expect("table encodes");
        let InterchangeValue::Table(restored) = decode(&encoded).expect("table decodes") else {
            panic!("expected table");
        };
        let values = restored
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("float column");
        assert!(values.is_valid(0));
        assert!(values.value(0).is_nan());
        assert!(values.is_null(1));
        assert_eq!(values.value(2), 3.5);
    }

    #[test]
    fn scalar_roundtrip_preserves_typed_missing_and_nan() {
        for value in [
            ScalarValue::Float64(None),
            ScalarValue::Float64(Some(f64::NAN)),
            ScalarValue::Int64(None),
            ScalarValue::Utf8(Some("text".into())),
            ScalarValue::Complex64(Some(Complex32 {
                real: 1.5,
                imaginary: -2.0,
            })),
            ScalarValue::Complex128(Some(Complex64 {
                real: 3.25,
                imaginary: -4.5,
            })),
            ScalarValue::Complex64(None),
            ScalarValue::Complex128(None),
        ] {
            let restored = decode(&encode(&InterchangeValue::Scalar(value.clone())).unwrap())
                .expect("scalar decodes");
            let InterchangeValue::Scalar(restored) = restored else {
                panic!("expected scalar");
            };
            match (value, restored) {
                (ScalarValue::Float64(Some(expected)), ScalarValue::Float64(Some(actual)))
                    if expected.is_nan() =>
                {
                    assert!(actual.is_nan());
                }
                (expected, actual) => assert_eq!(actual, expected),
            }
        }
    }

    #[test]
    fn tensor_roundtrip_preserves_column_major_shape_and_axes() {
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
        let restored = decode(&encode(&InterchangeValue::Tensor(tensor.clone())).unwrap())
            .expect("tensor decodes");
        let InterchangeValue::Tensor(restored) = restored else {
            panic!("expected tensor");
        };
        assert_eq!(restored, tensor);
    }

    #[test]
    fn complex_tensor_uses_real_and_imaginary_components() {
        let tensor = DenseTensor {
            shape: vec![2],
            axes: vec![],
            data: TensorData::Complex128(vec![
                Complex64 {
                    real: 1.0,
                    imaginary: -2.0,
                },
                Complex64 {
                    real: 3.5,
                    imaginary: 4.25,
                },
            ]),
        };
        let restored = decode(&encode(&InterchangeValue::Tensor(tensor.clone())).unwrap())
            .expect("tensor decodes");
        let InterchangeValue::Tensor(restored) = restored else {
            panic!("expected tensor");
        };
        assert_eq!(restored, tensor);
    }

    #[test]
    fn sparse_csc_roundtrip_uses_zero_based_int64_indices() {
        let matrix = CscMatrix {
            shape: [3, 2],
            indptr: vec![0, 2, 3],
            indices: vec![0, 2, 1],
            values: TensorData::Float64(vec![10.0, 20.0, 30.0]),
        };
        let encoded = encode(&InterchangeValue::SparseCsc(matrix.clone())).unwrap();
        let mut reader = StreamReader::try_new(Cursor::new(&encoded), None).unwrap();
        let metadata: Envelope = serde_json::from_str(
            reader
                .schema()
                .metadata()
                .get(PROFILE_METADATA_KEY)
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            metadata.value,
            ValueMetadata::SparseCsc { index_base: 0, .. }
        ));
        let batch = reader.next().unwrap().unwrap();
        for column in [batch.column(0), batch.column(1)] {
            let list = column.as_any().downcast_ref::<LargeListArray>().unwrap();
            assert_eq!(list.value_type(), DataType::Int64);
        }
        let InterchangeValue::SparseCsc(restored) = decode(&encoded).unwrap() else {
            panic!("expected sparse matrix");
        };
        assert_eq!(restored, matrix);
    }

    #[test]
    fn rejects_sparse_metadata_with_nonzero_wire_index_base() {
        let matrix = CscMatrix {
            shape: [1, 1],
            indptr: vec![0, 1],
            indices: vec![0],
            values: TensorData::Float64(vec![1.0]),
        };
        let encoded = encode(&InterchangeValue::SparseCsc(matrix)).unwrap();
        let invalid_index_base = rewrite_envelope(&encoded, |envelope| {
            if let ValueMetadata::SparseCsc { index_base, .. } = &mut envelope.value {
                *index_base = 1;
            }
        });
        assert!(matches!(
            decode(&invalid_index_base),
            Err(ProfileError::InvalidSparse(message))
                if message.contains("wire index base must be zero")
        ));
    }

    #[test]
    fn rejects_shape_mismatch_and_axis_metadata_mismatch() {
        let bad_shape = DenseTensor {
            shape: vec![2, 2],
            axes: vec![],
            data: TensorData::Float64(vec![1.0, 2.0, 3.0]),
        };
        assert!(matches!(
            encode(&InterchangeValue::Tensor(bad_shape)),
            Err(ProfileError::InvalidTensor(_))
        ));

        let bad_axes = DenseTensor {
            shape: vec![2],
            axes: vec![AxisMetadata::default(), AxisMetadata::default()],
            data: TensorData::Float64(vec![1.0, 2.0]),
        };
        assert!(matches!(
            encode(&InterchangeValue::Tensor(bad_axes)),
            Err(ProfileError::InvalidTensor(_))
        ));
    }

    #[test]
    fn rejects_unsupported_arrow_table_types() {
        let dictionary_type =
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8));
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "dictionary",
                dictionary_type.clone(),
                true,
            )])),
            vec![new_empty_array(&dictionary_type)],
        )
        .expect("empty dictionary batch is valid");
        assert!(matches!(
            encode(&InterchangeValue::Table(batch)),
            Err(ProfileError::UnsupportedType(_))
        ));
    }

    #[test]
    fn rejects_malformed_sparse_indices() {
        let matrix = CscMatrix {
            shape: [2, 1],
            indptr: vec![0, 2],
            indices: vec![1, 0],
            values: TensorData::Float64(vec![1.0, 2.0]),
        };
        assert!(matches!(
            encode(&InterchangeValue::SparseCsc(matrix)),
            Err(ProfileError::InvalidSparse(_))
        ));
    }

    #[test]
    fn rejects_unknown_profile_version_and_malformed_tensor_metadata() {
        let tensor = InterchangeValue::Tensor(DenseTensor {
            shape: vec![2],
            axes: vec![],
            data: TensorData::Float64(vec![1.0, 2.0]),
        });
        let encoded = encode(&tensor).unwrap();
        let unknown_version = rewrite_envelope(&encoded, |envelope| envelope.version = 2);
        assert!(matches!(
            decode(&unknown_version),
            Err(ProfileError::UnsupportedVersion(2))
        ));

        let wrong_shape = rewrite_envelope(&encoded, |envelope| {
            if let ValueMetadata::Tensor { shape, .. } = &mut envelope.value {
                *shape = vec![3];
            }
        });
        assert!(matches!(
            decode(&wrong_shape),
            Err(ProfileError::InvalidTensor(_))
        ));
    }

    #[test]
    fn rejects_batches_without_envelope() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1]))])
            .unwrap();
        let mut bytes = Vec::new();
        let mut writer = StreamWriter::try_new(&mut bytes, schema.as_ref()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        drop(writer);
        assert!(matches!(decode(&bytes), Err(ProfileError::MissingEnvelope)));
    }

    #[test]
    fn rejects_dense_tensor_nulls() {
        let schema = Arc::new(Schema::new_with_metadata(
            vec![Field::new("values", DataType::Float64, true)],
            std::collections::HashMap::from([(
                PROFILE_METADATA_KEY.to_owned(),
                serde_json::to_string(&Envelope {
                    profile: PROFILE_ID.to_owned(),
                    version: PROFILE_VERSION,
                    value: ValueMetadata::Tensor {
                        dtype: TensorDType::Float64,
                        shape: vec![2],
                        layout: COLUMN_MAJOR.to_owned(),
                        axes: vec![],
                    },
                })
                .unwrap(),
            )]),
        ));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(vec![Some(1.0), None]))],
        )
        .unwrap();
        let mut bytes = Vec::new();
        let mut writer = StreamWriter::try_new(&mut bytes, schema.as_ref()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        drop(writer);
        assert!(matches!(
            decode(&bytes),
            Err(ProfileError::InvalidTensor(_))
        ));
    }
}
