use crate::Datetime;
use indexmap::IndexMap;

/// An ordered configuration value shared by all input formats.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    Datetime(Datetime),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

impl Value {
    /// The kind used in configuration diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Integer(_) | Self::Unsigned(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Datetime(_) => "datetime",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Borrow the ordered fields of an object.
    pub fn as_object(&self) -> Option<&IndexMap<String, Value>> {
        if let Self::Object(map) = self {
            Some(map)
        } else {
            None
        }
    }

    /// Borrow the contents of a string.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(text) = self {
            Some(text)
        } else {
            None
        }
    }

    pub(crate) fn stringify(&self) -> Option<String> {
        Some(match self {
            Self::String(s) => s.clone(),
            Self::Integer(i) => i.to_string(),
            Self::Unsigned(u) => u.to_string(),
            Self::Float(f) => float(*f),
            Self::Bool(b) => b.to_string(),
            Self::Datetime(dt) => dt.to_string(),
            _ => return None,
        })
    }
}

fn float(f: f64) -> String {
    if f.is_finite() {
        format!("{f:?}")
    } else if f.is_nan() {
        "nan".into()
    } else if f.is_sign_negative() {
        "-inf".into()
    } else {
        "inf".into()
    }
}
