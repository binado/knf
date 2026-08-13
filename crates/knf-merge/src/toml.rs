//! TOML merge: tables recurse per key; every other kind replaces wholesale.

use crate::{MergeError, MergeOptions, strict};

/// The kind of a TOML value, for conflict reporting and parse errors.
///
/// Integers and floats are one kind: an int layer overriding a float (or the
/// reverse) is a routine thing to write and carries no risk of shadowing a
/// subtree, which is what strict mode exists to catch. Datetimes are their own
/// kind — they are not strings until JSON conversion.
pub fn toml_kind(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::Table(_) => "object",
        toml::Value::Array(_) => "array",
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) | toml::Value::Float(_) => "number",
        toml::Value::Boolean(_) => "bool",
        toml::Value::Datetime(_) => "datetime",
    }
}

/// Merges `over` into `base` in place.
///
/// Tables recurse per key. Arrays, scalars and datetimes all replace wholesale —
/// notably arrays are never index-merged or concatenated.
pub fn merge_toml(
    base: &mut toml::Value,
    over: toml::Value,
    opts: &MergeOptions,
) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_toml_at(base, over, opts, &mut path)
}

/// Folds a list of layers into one document, seeded with an empty table.
///
/// The fold must be strictly left over the *flat* layer list. Merge is not
/// associative — any scalar shadowing a table breaks it:
///
/// ```text
/// {a:{b:1}} + {a:5} + {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
///
/// So callers must never merge subgroups and then combine the results.
/// Flatten first, fold second.
pub fn merge_all_toml(
    layers: impl IntoIterator<Item = toml::Value>,
    opts: &MergeOptions,
) -> Result<toml::Value, MergeError> {
    let mut acc = toml::Value::Table(toml::Table::new());
    let mut path = Vec::new();
    for layer in layers {
        merge_toml_at(&mut acc, layer, opts, &mut path)?;
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    Ok(acc)
}

/// The recursive worker. `path` is a breadcrumb threaded by push/pop so that a
/// conflict can report where it happened without every frame allocating.
fn merge_toml_at(
    base: &mut toml::Value,
    over: toml::Value,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    match (base, over) {
        (toml::Value::Table(base_map), toml::Value::Table(over_map)) => {
            for (k, v) in over_map {
                if let Some(slot) = base_map.get_mut(&k) {
                    path.push(k);
                    merge_toml_at(slot, v, opts, path)?;
                    path.pop();
                } else {
                    base_map.insert(k, v);
                }
            }
            Ok(())
        }
        (base, over) => replace_toml(base, over, opts, path),
    }
}

fn replace_toml(
    base: &mut toml::Value,
    over: toml::Value,
    opts: &MergeOptions,
    path: &[String],
) -> Result<(), MergeError> {
    if opts.strict {
        strict::check(toml_kind(base), toml_kind(&over), path)?;
    }
    *base = over;
    Ok(())
}
