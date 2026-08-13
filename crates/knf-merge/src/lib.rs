//! Layered merge over [`serde_json::Value`].
//!
//! This crate is deliberately tiny and dependency-free beyond `serde_json` and
//! `thiserror`. It knows nothing about files, formats, or the command line;
//! provenance (which file wrote a value) is the caller's job.

use serde_json::{Map, Value};

mod strict;

pub use strict::kind;

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
pub fn merge(base: &mut Value, over: Value, opts: &MergeOptions) -> Result<(), MergeError> {
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
/// So callers must never merge subgroups and then combine the results, however
/// tempting that looks when implementing something like `matrix`. Flatten
/// first, fold second.
pub fn merge_all(
    layers: impl IntoIterator<Item = Value>,
    opts: &MergeOptions,
) -> Result<Value, MergeError> {
    let mut acc = Value::Object(Map::new());
    let mut path = Vec::new();
    for layer in layers {
        merge_at(&mut acc, layer, opts, &mut path)?;
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    Ok(acc)
}

/// The recursive worker. `path` is a breadcrumb threaded by push/pop so that a
/// conflict can report where it happened without every frame allocating.
fn merge_at(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.entry(k) {
                    // A key absent from the base is an insert, never a
                    // conflict — strict mode is about *changing* a value's
                    // kind, not about adding new ones.
                    serde_json::map::Entry::Vacant(slot) => {
                        slot.insert(v);
                    }
                    serde_json::map::Entry::Occupied(mut slot) => {
                        path.push(slot.key().clone());
                        merge_at(slot.get_mut(), v, opts, path)?;
                        path.pop();
                    }
                }
            }
            Ok(())
        }
        (b, o) => {
            if opts.strict {
                strict::check(b, &o, path)?;
            }
            *b = o;
            Ok(())
        }
    }
}
