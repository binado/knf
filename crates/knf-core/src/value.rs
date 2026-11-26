use std::fmt;

use crate::Datetime;
use indexmap::IndexMap;

/// An ordered configuration value shared by all input formats.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Unsigned(Unsigned),
    Float(f64),
    String(String),
    Datetime(Datetime),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

/// A [`Value`]'s type, as named in configuration diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Null,
    Bool,
    /// Any integer or float.
    Number,
    String,
    Datetime,
    Array,
    Object,
}

impl ValueKind {
    /// The indefinite article the name takes: `an object`, `a null`.
    pub fn article(self) -> &'static str {
        match self {
            Self::Array | Self::Object => "an",
            Self::Null | Self::Bool | Self::Number | Self::String | Self::Datetime => "a",
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::String => "string",
            Self::Datetime => "datetime",
            Self::Array => "array",
            Self::Object => "object",
        })
    }
}

/// An integer above `i64::MAX`, built by [`Value::unsigned`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsigned(u64);

impl Unsigned {
    /// The integer value.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl Value {
    /// An unsigned integer, normalized to [`Value::Integer`] when it fits in `i64`.
    pub fn unsigned(value: u64) -> Self {
        i64::try_from(value).map_or(Self::Unsigned(Unsigned(value)), Self::Integer)
    }

    /// The kind used in configuration diagnostics.
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Null => ValueKind::Null,
            Self::Bool(_) => ValueKind::Bool,
            Self::Integer(_) | Self::Unsigned(_) | Self::Float(_) => ValueKind::Number,
            Self::String(_) => ValueKind::String,
            Self::Datetime(_) => ValueKind::Datetime,
            Self::Array(_) => ValueKind::Array,
            Self::Object(_) => ValueKind::Object,
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
            Self::Unsigned(u) => u.get().to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stringify_spells_scalars_canonically() {
        let spelled = |value: Value| value.stringify();
        assert_eq!(spelled(Value::Bool(true)).as_deref(), Some("true"));
        assert_eq!(spelled(Value::Float(1.0)).as_deref(), Some("1.0"));
        assert_eq!(spelled(Value::Float(-0.0)).as_deref(), Some("-0.0"));
        assert_eq!(spelled(Value::Float(1e300)).as_deref(), Some("1e300"));
        assert_eq!(spelled(Value::Float(1e-7)).as_deref(), Some("1e-7"));
        assert_eq!(spelled(Value::Float(f64::INFINITY)).as_deref(), Some("inf"));
        assert_eq!(
            spelled(Value::Float(f64::NEG_INFINITY)).as_deref(),
            Some("-inf")
        );
        assert_eq!(spelled(Value::Float(f64::NAN)).as_deref(), Some("nan"));
        assert_eq!(spelled(Value::Integer(-3)).as_deref(), Some("-3"));
        assert_eq!(
            spelled(Value::unsigned(u64::MAX)).as_deref(),
            Some("18446744073709551615")
        );
        assert_eq!(
            spelled("1979-05-27T07:32:00Z".parse().map(Value::Datetime).unwrap()).as_deref(),
            Some("1979-05-27T07:32:00Z")
        );
    }

    #[test]
    fn stringify_rejects_null_and_containers() {
        assert_eq!(Value::Null.stringify(), None);
        assert_eq!(Value::Array(vec![]).stringify(), None);
        assert_eq!(Value::Object(IndexMap::new()).stringify(), None);
    }

    #[test]
    fn kind_names_every_variant() {
        let kinds = [
            (Value::Null, "null"),
            (Value::Bool(true), "bool"),
            (Value::Integer(1), "number"),
            (Value::unsigned(u64::MAX), "number"),
            (Value::Float(1.0), "number"),
            (Value::String("a".into()), "string"),
            (Value::Datetime("1979-05-27".parse().unwrap()), "datetime"),
            (Value::Array(vec![]), "array"),
            (Value::Object(IndexMap::new()), "object"),
        ];
        for (value, kind) in kinds {
            assert_eq!(value.kind().to_string(), kind);
        }
    }

    #[test]
    fn unsigned_normalizes_to_integer_when_it_fits() {
        assert_eq!(Value::unsigned(5), Value::Integer(5));
        assert_eq!(Value::unsigned(i64::MAX as u64), Value::Integer(i64::MAX));
        assert_eq!(
            Value::unsigned(i64::MAX as u64 + 1),
            Value::Unsigned(Unsigned(i64::MAX as u64 + 1))
        );
    }

    #[test]
    fn accessors_match_only_their_variant() {
        assert_eq!(Value::String("a".into()).as_str(), Some("a"));
        assert_eq!(Value::Integer(1).as_str(), None);
        let object = Value::Object([("k".into(), Value::Null)].into());
        assert_eq!(object.as_object().map(IndexMap::len), Some(1));
        assert!(Value::Null.as_object().is_none());
    }
}
