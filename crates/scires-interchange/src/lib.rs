//! Versioned Arrow IPC value profile for explicit SciRes kernel transfers.
//!
//! The crate keeps Arrow serialization outside the domain crates and defines the
//! supported v1 payloads: standard Arrow scalars and tables, column-major dense
//! tensors, and zero-based CSC sparse matrices.
//!
//! # Examples
//!
//! ```
//! use scires_interchange::{decode, encode, DenseTensor, InterchangeValue, TensorData};
//!
//! let value = InterchangeValue::Tensor(DenseTensor {
//!     shape: vec![2, 2],
//!     axes: vec![],
//!     data: TensorData::Float64(vec![1.0, 2.0, 3.0, 4.0]),
//! });
//! let bytes = encode(&value)?;
//! let restored = decode(&bytes)?;
//! assert!(matches!(restored, InterchangeValue::Tensor(_)));
//! # Ok::<(), scires_interchange::ProfileError>(())
//! ```

mod profile;

pub use profile::{
    decode, encode, AxisMetadata, Complex32, Complex64, CscMatrix, DenseTensor, InterchangeValue,
    ProfileError, ScalarValue, TensorDType, TensorData, PROFILE_ID, PROFILE_METADATA_KEY,
    PROFILE_VERSION,
};
