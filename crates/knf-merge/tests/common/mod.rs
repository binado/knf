//! Dev-only bridge from JSON literals to the IR.
//!
//! The case table's readability comes from writing layers as JSON strings, and
//! `serde_json` is a dev-dependency so this costs consumers nothing. It
//! duplicates ~20 lines of `knf/src/value.rs` on purpose: the merge tests must
//! not depend on the binary crate.

use knf_merge::{Number, Value};

/// Parses a JSON literal into the IR. Panics on a malformed literal — every
/// caller passes a `&'static str` written in the test file.
pub fn ir(s: &str) -> Value {
    let json: serde_json::Value =
        serde_json::from_str(s).unwrap_or_else(|e| panic!("bad JSON literal `{s}`: {e}"));
    from_json(json)
}

fn from_json(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(if let Some(i) = n.as_i64() {
            Number::I64(i)
        } else if let Some(u) = n.as_u64() {
            Number::from_u64(u)
        } else {
            Number::F64(n.as_f64().expect("a JSON number is i64, u64 or f64"))
        }),
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(items) => Value::Array(items.into_iter().map(from_json).collect()),
        serde_json::Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, from_json(v))).collect())
        }
    }
}
