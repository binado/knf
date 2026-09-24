//! Layered merge over an owned [`Value`] tree.
//!
//! One walk, one value type. Every format parses into [`Value`] before merging,
//! so JSON and TOML layers stack without a conversion in the middle. This crate
//! knows nothing about files, formats or the command line; parsing, emission and
//! provenance are all the caller's job.
//!
//! `thiserror` and `indexmap` are the only dependencies — neither is a format
//! crate. `cargo tree -p knf-core --depth 1` is the enforcement.
//!
//! The path vocabulary lives here too ([`Seg`], [`RefPath`]), one module over.
//! It is pure text handling and needs nothing this crate does not already have,
//! and putting it here is what keeps one spelling and one renderer for the key
//! paths that both the merge errors and `${...}` references have to display.

mod path;
mod strict;
mod value;

pub use path::{PathError, RefPath, Seg, render_path};
pub use value::{Map, Number, Value};

use path::render_keys;

/// Knobs on the merge itself. Passed by reference rather than encoded as cargo
/// features: features are additive and unify across a dependency graph, so a
/// `strict` feature would silently change behaviour for one consumer the moment
/// a second consumer enabled it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Error when a layer changes the kind of an existing key.
    pub strict: bool,
    /// Replace top-level keys wholesale instead of recursing into them — jq's
    /// `a + b` rather than `a * b`.
    pub shallow: bool,
}

impl MergeOptions {
    /// The default: deep merge, last layer wins, no type checking.
    pub const LAST_WINS: Self = Self {
        strict: false,
        shallow: false,
    };
    /// Error when a layer changes the kind of an existing key.
    pub const STRICT: Self = Self {
        strict: true,
        shallow: false,
    };
    /// Top-level keys only: a later layer's value replaces the earlier one whole.
    pub const SHALLOW: Self = Self {
        strict: false,
        shallow: true,
    };
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// A layer replaced an existing key with a value of a different kind.
    ///
    /// Carries a key path and nothing else — no filenames, no layer indices.
    #[error(
        "type conflict at `{}`: {expected} would be replaced by {found}",
        render_keys(path)
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

/// Merges `over` into `base` in place.
///
/// Objects recurse per key. Arrays, scalars, datetimes and null all replace
/// wholesale — notably arrays are never index-merged or concatenated, and null
/// is an ordinary value that overwrites rather than a delete instruction. This
/// is jq's `a * b`.
///
/// Under [`MergeOptions::shallow`] only the top level is merged key by key;
/// every colliding value is replaced whole, objects included — jq's `a + b`.
pub fn merge_into(base: &mut Value, over: Value, opts: &MergeOptions) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_at(base, over, opts, &mut path)
}

/// Folds a list of layers into one document, last-wins, seeded with an empty object.
///
/// Equivalent to [`merge_with`] using [`MergeOptions::LAST_WINS`].
pub fn merge(layers: impl IntoIterator<Item = Value>) -> Result<Value, MergeError> {
    merge_with(layers, &MergeOptions::LAST_WINS)
}

/// Folds a list of layers into one document, seeded with an empty object.
///
/// The fold must be strictly left over the *flat* layer list. The deep merge is
/// not associative — any scalar shadowing an object breaks it:
///
/// ```text
/// {a:{b:1}} * {a:5} * {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
///
/// So callers must never merge subgroups and then combine the results.
/// Flatten first, fold second. (The shallow merge happens to be associative,
/// but the fold does not rely on it.)
pub fn merge_with(
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
///
/// Shallow mode needs no depth counter: it replaces at the first collision
/// below the root, so the walk never gets deeper than one level.
fn merge_at(
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
                    if opts.shallow {
                        replace(slot, v, opts, path)?;
                    } else {
                        merge_at(slot, v, opts, path)?;
                    }
                    path.pop();
                } else {
                    base_map.insert(k, v);
                }
            }
            Ok(())
        }
        (base, over) => replace(base, over, opts, path),
    }
}

fn replace(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &[String],
) -> Result<(), MergeError> {
    if opts.strict {
        strict::check(base.kind(), over.kind(), path)?;
    }
    *base = over;
    Ok(())
}
