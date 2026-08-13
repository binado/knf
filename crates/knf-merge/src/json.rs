//! JSON merge: objects recurse per key; every other kind replaces wholesale.

use serde_json::{Map, Value};

use crate::{MergeError, MergeOptions, strict};

/// The kind of a JSON value, for conflict reporting and parse errors.
///
/// All numbers are one kind: an int layer overriding a float (or the reverse) is
/// a routine thing to write and carries no risk of shadowing a subtree, which is
/// what strict mode exists to catch.
pub fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Object(_) => "object",
        Value::Array(_) => "array",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "bool",
        Value::Null => "null",
    }
}

/// Merges `over` into `base` in place.
///
/// Objects recurse per key. Arrays, scalars and null all replace wholesale —
/// notably arrays are never index-merged or concatenated, and null is an
/// ordinary value that overwrites rather than a delete instruction.
pub fn merge_json(base: &mut Value, over: Value, opts: &MergeOptions) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_json_at(base, over, opts, &mut path)
}

/// Folds a list of layers into one document, seeded with an empty object.
///
/// The fold must be strictly left over the *flat* layer list. Merge is not
/// associative — any scalar shadowing an object breaks it:
///
/// ```text
/// {a:{b:1}} + {a:5} + {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
///
/// So callers must never merge subgroups and then combine the results.
/// Flatten first, fold second.
pub fn merge_all_json(
    layers: impl IntoIterator<Item = Value>,
    opts: &MergeOptions,
) -> Result<Value, MergeError> {
    let mut acc = Value::Object(Map::new());
    let mut path = Vec::new();
    for layer in layers {
        merge_json_at(&mut acc, layer, opts, &mut path)?;
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    Ok(acc)
}

/// The recursive worker. `path` is a breadcrumb threaded by push/pop so that a
/// conflict can report where it happened without every frame allocating.
fn merge_json_at(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    match (base, over) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (k, v) in over_map {
                if let Some(slot) = base_map.get_mut(&k) {
                    path.push(k);
                    merge_json_at(slot, v, opts, path)?;
                    path.pop();
                } else {
                    base_map.insert(k, v);
                }
            }
            Ok(())
        }
        (base, over) => replace_json(base, over, opts, path),
    }
}

fn replace_json(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &[String],
) -> Result<(), MergeError> {
    if opts.strict {
        strict::check(json_kind(base), json_kind(&over), path)?;
    }
    *base = over;
    Ok(())
}
