//! The owned value tree every layer is merged as.
//!
//! Deliberately a superset of JSON and TOML rather than either one: [`Null`] is
//! JSON-only, [`Datetime`] is TOML-only, and both survive the merge untouched so
//! that the format crates are only involved at the parse and emit boundaries.
//!
//! [`Null`]: Value::Null
//! [`Datetime`]: Value::Datetime

/// The object type. `indexmap` rather than `BTreeMap` so input key order
/// survives, and rather than `Vec<(String, Value)>` so merge's per-key lookup is
/// not quadratic.
pub type Map = indexmap::IndexMap<String, Value>;

/// A parsed document, or any node within one.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    /// An RFC 3339-ish TOML datetime, kept as its source spelling.
    ///
    /// Every datetime originates in the TOML parser. It round-trips exactly through
    /// `Display`/`FromStr` for all four TOML forms (offset datetime, local
    /// datetime, local date, local time), so a string is enough to carry it
    /// across a merge without pulling `toml` into this crate. JSON has no
    /// datetime, so it renders as a string on the way out.
    Datetime(String),
    Array(Vec<Value>),
    Object(Map),
}

/// A number, kept in the widest lossless representation of its source.
///
/// Three variants rather than a single `f64`: JSON integers above [`i64::MAX`]
/// (snowflake IDs, hashes) are real and must round-trip exactly, and `f64`
/// silently rounds them.
///
/// `U64` is reserved for values that do not fit an `i64`; construct through
/// [`Number::from_u64`] to keep that canonical. Without it, derived
/// [`PartialEq`] would make `I64(1) != U64(1)` and equality would depend on
/// which parser produced the value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    I64(i64),
    U64(u64),
    F64(f64),
}

impl Number {
    /// Demotes to [`I64`](Number::I64) when the value fits, so that every
    /// representable integer has exactly one representation.
    pub fn from_u64(u: u64) -> Self {
        match i64::try_from(u) {
            Ok(i) => Self::I64(i),
            Err(_) => Self::U64(u),
        }
    }
}

impl From<i64> for Number {
    fn from(i: i64) -> Self {
        Self::I64(i)
    }
}

impl From<u64> for Number {
    fn from(u: u64) -> Self {
        Self::from_u64(u)
    }
}

impl From<f64> for Number {
    fn from(f: f64) -> Self {
        Self::F64(f)
    }
}

impl Value {
    /// The kind of a value, for conflict reporting and parse errors.
    ///
    /// All numbers are one kind: an int layer overriding a float (or the
    /// reverse) is a routine thing to write and carries no risk of shadowing a
    /// subtree, which is what strict mode exists to catch. Datetimes are their
    /// own kind — they are not strings until JSON conversion.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Object(_) => "object",
            Self::Array(_) => "array",
            Self::String(_) => "string",
            Self::Datetime(_) => "datetime",
            Self::Number(_) => "number",
            Self::Bool(_) => "bool",
            Self::Null => "null",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u64_that_fits_normalises_to_i64() {
        assert_eq!(Number::from_u64(1), Number::I64(1));
        assert_eq!(Number::from_u64(i64::MAX as u64), Number::I64(i64::MAX));
        assert_eq!(Number::from_u64(u64::MAX), Number::U64(u64::MAX));
    }

    #[test]
    fn int_and_float_share_a_kind_but_datetime_does_not() {
        assert_eq!(Value::Number(Number::I64(1)).kind(), "number");
        assert_eq!(Value::Number(Number::F64(1.5)).kind(), "number");
        assert_eq!(Value::String("x".into()).kind(), "string");
        assert_eq!(
            Value::Datetime("1979-05-27T07:32:00Z".into()).kind(),
            "datetime"
        );
    }
}
