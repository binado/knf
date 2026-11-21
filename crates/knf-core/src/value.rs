use crate::Datetime;
use indexmap::IndexMap;

/// An ordered configuration value shared by all input formats.
///
/// Integers that fit in `i64` are [`Value::Integer`]; [`Value::Unsigned`] holds
/// only larger ones. Build unsigned values with [`Value::unsigned`] to keep
/// that split. JSON never yields `Datetime`, and TOML never yields `Null`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    /// An integer above `i64::MAX`.
    Unsigned(u64),
    Float(f64),
    String(String),
    Datetime(Datetime),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

impl Value {
    /// An unsigned integer, normalized to [`Value::Integer`] when it fits in `i64`.
    pub fn unsigned(value: u64) -> Self {
        i64::try_from(value).map_or(Self::Unsigned(value), Self::Integer)
    }

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

    /// Mutably borrow the ordered fields of an object.
    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Value>> {
        if let Self::Object(map) = self {
            Some(map)
        } else {
            None
        }
    }

    /// Borrow the items of an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Self::Array(items) = self {
            Some(items)
        } else {
            None
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    /// The integer, if this is an [`Value::Integer`].
    pub fn as_i64(&self) -> Option<i64> {
        if let Self::Integer(i) = self {
            Some(*i)
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

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Self::unsigned(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}

impl From<IndexMap<String, Value>> for Value {
    fn from(value: IndexMap<String, Value>) -> Self {
        Self::Object(value)
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
            spelled(Value::Unsigned(u64::MAX)).as_deref(),
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
            (Value::Unsigned(u64::MAX), "number"),
            (Value::Float(1.0), "number"),
            (Value::from("a"), "string"),
            (Value::Datetime("1979-05-27".parse().unwrap()), "datetime"),
            (Value::Array(vec![]), "array"),
            (Value::Object(IndexMap::new()), "object"),
        ];
        for (value, kind) in kinds {
            assert_eq!(value.kind(), kind);
        }
    }

    #[test]
    fn unsigned_normalizes_to_integer_when_it_fits() {
        assert_eq!(Value::unsigned(5), Value::Integer(5));
        assert_eq!(Value::unsigned(i64::MAX as u64), Value::Integer(i64::MAX));
        assert_eq!(
            Value::unsigned(i64::MAX as u64 + 1),
            Value::Unsigned(i64::MAX as u64 + 1)
        );
        assert_eq!(Value::from(7u64), Value::Integer(7));
    }

    #[test]
    fn accessors_match_only_their_variant() {
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Integer(1).as_bool(), None);
        assert_eq!(Value::Integer(9).as_i64(), Some(9));
        assert_eq!(Value::Unsigned(u64::MAX).as_i64(), None);
        assert_eq!(
            Value::Array(vec![Value::Null]).as_array().map(<[_]>::len),
            Some(1)
        );
        let mut object = Value::Object(IndexMap::new());
        object
            .as_object_mut()
            .unwrap()
            .insert("k".into(), Value::Null);
        assert_eq!(object.as_object().map(IndexMap::len), Some(1));
        assert!(Value::Null.as_object_mut().is_none());
    }
}
