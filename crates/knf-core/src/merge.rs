//! Layered configuration merge.

use crate::glob::KeyGlobPattern;
use crate::{Seg, Value};

/// Matches `path` or any ancestor against `glob`; paths containing an index never match.
pub(crate) fn shallow_at(glob: Option<&KeyGlobPattern>, path: &[Seg]) -> bool {
    let Some(glob) = glob else {
        return false;
    };
    let Some(keys) = path
        .iter()
        .map(|seg| match seg {
            Seg::Key(key) => Some(key.clone()),
            Seg::Index(_) => None,
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    (1..=keys.len()).any(|end| glob.matches_keys(&keys[..end]))
}

/// Merge options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Key paths to replace wholesale: `*` (top level), `foo`, `foo.*`.
    /// `None` is a deep merge.
    pub shallow: Option<KeyGlobPattern>,
}

impl MergeOptions {
    /// Top-level keys only: a later layer's value replaces the earlier one whole.
    pub fn shallow_root() -> Self {
        Self {
            shallow: Some("*".parse().expect("valid root selector")),
        }
    }
}

/// Merges `over` into `base` in place (jq's `a * b`).
///
/// Objects recurse per key; everything else, including arrays and null,
/// replaces. Paths matched by [`MergeOptions::shallow`] replace wholesale.
pub fn merge_into(base: &mut Value, over: Value, opts: &MergeOptions) {
    let mut path = Vec::new();
    merge_at(base, over, opts, &mut path);
}

/// Left-folds layers into one document, starting from an empty object.
///
/// Merging is not associative (a scalar layer shadows earlier objects):
///
/// ```text
/// {a:{b:1}} * {a:5} * {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
pub fn merge(layers: impl IntoIterator<Item = Value>, opts: &MergeOptions) -> Value {
    let mut acc = Value::Object(Default::default());
    let mut path = Vec::new();
    for layer in layers {
        merge_at(&mut acc, layer, opts, &mut path);
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    acc
}

fn merge_at(base: &mut Value, over: Value, opts: &MergeOptions, path: &mut Vec<String>) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                if let Some(slot) = base.get_mut(&key) {
                    path.push(key);
                    if opts
                        .shallow
                        .as_ref()
                        .is_some_and(|glob| glob.matches_keys(path))
                    {
                        *slot = value;
                    } else {
                        merge_at(slot, value, opts, path);
                    }
                    path.pop();
                } else {
                    base.insert(key, value);
                }
            }
        }
        (slot, value) => *slot = value,
    }
}
