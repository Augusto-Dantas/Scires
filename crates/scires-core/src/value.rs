//! Language-neutral representation of workspace values (for the variable explorer).

use crate::Language;
use serde::{Deserialize, Serialize};

/// Element type of an array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DType {
    /// Boolean.
    Bool,
    /// Signed integer of the given bit width.
    Int(u8),
    /// Unsigned integer of the given bit width.
    UInt(u8),
    /// Floating point of the given bit width.
    Float(u8),
    /// Complex with float components of the given bit width.
    Complex(u8),
    /// String / text.
    Str,
    /// Any other element type.
    Object,
}

/// A summary of a value living inside a kernel. Large data is never copied here;
/// only metadata and a short preview are kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Value {
    /// Absence of value (`nothing`, `None`, `NULL`).
    Nil,
    /// Boolean scalar.
    Bool(bool),
    /// Integer scalar.
    Int(i64),
    /// Floating point scalar.
    Float(f64),
    /// Complex scalar.
    Complex {
        /// Real part.
        re: f64,
        /// Imaginary part.
        im: f64,
    },
    /// Text.
    Str(String),
    /// N-dimensional array (column-major, like Julia/MATLAB).
    Array {
        /// Dimensions.
        shape: Vec<usize>,
        /// Element type.
        dtype: DType,
        /// Up to N leading elements as text.
        preview: Vec<String>,
    },
    /// Tabular data (DataFrame / data.frame / pandas).
    Table {
        /// Column names and types.
        columns: Vec<(String, DType)>,
        /// Number of rows.
        rows: usize,
    },
    /// Anything else, shown by its type name and textual summary.
    Opaque {
        /// Type name in the source language.
        type_name: String,
        /// Short human-readable representation.
        summary: String,
    },
}

impl Value {
    /// Human-readable type label, used by the variable explorer.
    pub fn type_label(&self) -> String {
        match self {
            Self::Nil => "Nothing".into(),
            Self::Bool(_) => "Bool".into(),
            Self::Int(_) => "Int64".into(),
            Self::Float(_) => "Float64".into(),
            Self::Complex { .. } => "ComplexF64".into(),
            Self::Str(_) => "String".into(),
            Self::Array { shape, dtype, .. } => format!("Array{{{dtype:?}, {}}}", shape.len()),
            Self::Table { .. } => "Table".into(),
            Self::Opaque { type_name, .. } => type_name.clone(),
        }
    }

    /// Shape as MATLAB-style size (scalars are `1x1`).
    pub fn size(&self) -> Vec<usize> {
        match self {
            Self::Array { shape, .. } => shape.clone(),
            Self::Table { columns, rows } => vec![*rows, columns.len()],
            Self::Str(s) => vec![1, s.chars().count()],
            _ => vec![1, 1],
        }
    }
}

/// A named value owned by a specific kernel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    /// Identifier in the source language.
    pub name: String,
    /// Kernel language that owns the value.
    pub language: Language,
    /// Value summary.
    pub value: Value,
}

impl Variable {
    /// Create a new variable.
    pub fn new(name: impl Into<String>, language: Language, value: Value) -> Self {
        Self {
            name: name.into(),
            language,
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_label() {
        let a = Value::Array {
            shape: vec![3, 4],
            dtype: DType::Float(64),
            preview: vec![],
        };
        assert_eq!(a.size(), vec![3, 4]);
        assert_eq!(Value::Float(1.0).size(), vec![1, 1]);
        assert_eq!(Value::Int(1).type_label(), "Int64");
    }
}
