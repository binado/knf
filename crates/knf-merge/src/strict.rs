//! Type-conflict detection for `--strict`.
//!
//! Strict mode catches the class of mistake where a leaf accidentally shadows a
//! subtree. Pleasant side effect: it rejects exactly the type changes that break
//! associativity, so under strict mode the merge *is* associative.

use serde_json::Value;

use crate::MergeError;

/// The kind of a value, for conflict reporting.
///
/// All numbers are one kind: an int layer overriding a float (or the reverse) is
/// a routine thing to write and carries no risk of shadowing a subtree, which is
/// what strict mode exists to catch.
pub fn kind(v: &Value) -> &'static str {
    match v {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "bool",
        Value::Null => "null",
    }
}

/// Errors if `over` would change the kind of the existing value `base`.
pub fn check(base: &Value, over: &Value, path: &[String]) -> Result<(), MergeError> {
    let (expected, found) = (kind(base), kind(over));
    if expected == found {
        return Ok(());
    }
    Err(MergeError::TypeConflict {
        path: path.to_vec(),
        expected,
        found,
    })
}
