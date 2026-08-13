//! Layered merge over JSON-like value trees.
//!
//! This crate is deliberately tiny and dependency-free beyond `serde_json` and
//! `thiserror`. It knows nothing about files, formats, or the command line;
//! provenance (which file wrote a value) is the caller's job.
//!
//! The algorithm is generic over [`MergeValue`]. `serde_json::Value` is the
//! built-in impl; other formats provide their own in the caller.

use serde_json::{Map, Value};

mod strict;

/// A JSON-like tree the merge algorithm can recurse over.
///
/// Objects recurse per key. Every other kind — arrays, scalars, null,
/// datetimes — replaces wholesale.
pub trait MergeValue: Sized {
    fn empty_object() -> Self;
    fn kind(&self) -> &'static str;
    fn is_object(&self) -> bool {
        self.kind() == "object"
    }
    /// Consume `self` as an object. `Err(self)` if it is not one.
    fn into_object(self) -> Result<Vec<(String, Self)>, Self>;
    /// Only called when `self` is an object.
    fn get_mut(&mut self, key: &str) -> Option<&mut Self>;
    /// Only called when `self` is an object.
    fn insert(&mut self, key: String, value: Self);
}

/// The kind of a JSON value, for conflict reporting and parse errors.
///
/// All numbers are one kind: an int layer overriding a float (or the reverse) is
/// a routine thing to write and carries no risk of shadowing a subtree, which is
/// what strict mode exists to catch.
pub fn kind(v: &Value) -> &'static str {
    MergeValue::kind(v)
}

impl MergeValue for Value {
    fn empty_object() -> Self {
        Value::Object(Map::new())
    }

    fn kind(&self) -> &'static str {
        match self {
            Value::Object(_) => "object",
            Value::Array(_) => "array",
            Value::String(_) => "string",
            Value::Number(_) => "number",
            Value::Bool(_) => "bool",
            Value::Null => "null",
        }
    }

    fn into_object(self) -> Result<Vec<(String, Self)>, Self> {
        match self {
            Value::Object(map) => Ok(map.into_iter().collect()),
            other => Err(other),
        }
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut Self> {
        self.as_object_mut()?.get_mut(key)
    }

    fn insert(&mut self, key: String, value: Self) {
        let Value::Object(map) = self else {
            debug_assert!(false, "insert called on a non-object");
            return;
        };
        map.insert(key, value);
    }
}

/// Knobs on the merge itself. Passed by reference rather than encoded as cargo
/// features: features are additive and unify across a dependency graph, so a
/// `strict` feature would silently change behaviour for one consumer the moment
/// a second consumer enabled it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeOptions {
    pub strict: bool,
}

impl MergeOptions {
    /// The default: last layer wins, no type checking.
    pub const LAST_WINS: Self = Self { strict: false };
    /// Error when a layer changes the kind of an existing key.
    pub const STRICT: Self = Self { strict: true };
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// A layer replaced an existing key with a value of a different kind.
    ///
    /// Carries a key path and nothing else — no filenames, no layer indices.
    #[error(
        "type conflict at `{}`: {expected} would be replaced by {found}",
        render_path(path)
    )]
    TypeConflict {
        path: Vec<String>,
        expected: &'static str,
        found: &'static str,
    },
}

impl MergeError {
    /// The dotted key path the conflict occurred at.
    pub fn path(&self) -> &[String] {
        match self {
            Self::TypeConflict { path, .. } => path,
        }
    }
}

/// Renders a key path for display. An empty path is the document root.
fn render_path(path: &[String]) -> String {
    if path.is_empty() {
        "<root>".to_string()
    } else {
        path.join(".")
    }
}

/// Merges `over` into `base` in place.
///
/// Objects recurse per key. Arrays, scalars and null all replace wholesale —
/// notably arrays are never index-merged or concatenated, and null is an
/// ordinary value that overwrites rather than a delete instruction.
pub fn merge<V: MergeValue>(base: &mut V, over: V, opts: &MergeOptions) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_at(base, over, opts, &mut path)
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
pub fn merge_all<V: MergeValue>(
    layers: impl IntoIterator<Item = V>,
    opts: &MergeOptions,
) -> Result<V, MergeError> {
    let mut acc = V::empty_object();
    let mut path = Vec::new();
    for layer in layers {
        merge_at(&mut acc, layer, opts, &mut path)?;
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    Ok(acc)
}

/// The recursive worker. `path` is a breadcrumb threaded by push/pop so that a
/// conflict can report where it happened without every frame allocating.
fn merge_at<V: MergeValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    if base.is_object() {
        match over.into_object() {
            Ok(pairs) => {
                for (k, v) in pairs {
                    if let Some(slot) = base.get_mut(&k) {
                        path.push(k);
                        merge_at(slot, v, opts, path)?;
                        path.pop();
                    } else {
                        base.insert(k, v);
                    }
                }
                return Ok(());
            }
            Err(over) => return replace(base, over, opts, path),
        }
    }
    replace(base, over, opts, path)
}

fn replace<V: MergeValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
    path: &[String],
) -> Result<(), MergeError> {
    if opts.strict {
        strict::check(base, &over, path)?;
    }
    *base = over;
    Ok(())
}
