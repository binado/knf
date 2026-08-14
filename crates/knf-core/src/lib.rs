//! Layered merge over an owned [`Value`] tree.
//!
//! One walk, one value type. Every format parses into [`Value`] before merging,
//! so JSON and TOML layers stack without a conversion in the middle. This crate
//! knows nothing about files, formats or the command line; parsing, emission and
//! provenance are all the caller's job.
//!
//! `thiserror` and `indexmap` are the only dependencies — neither is a format
//! crate. `cargo tree -p knf-core --depth 1` is the enforcement.

mod rules;
mod strict;
mod value;

pub use rules::{RuleError, RuleErrors, Rules, Strategy};
pub use value::{Map, Number, Value};

/// Knobs on the merge itself. Passed by reference rather than encoded as cargo
/// features: features are additive and unify across a dependency graph, so a
/// `strict` feature would silently change behaviour for one consumer the moment
/// a second consumer enabled it.
///
/// Not `Copy`, because [`Rules`] is not — but `None` is const-constructible
/// whatever it wraps, so the constants below survive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    pub strict: bool,
    /// Per-path overrides of the default merge. `None` is the default merge
    /// everywhere.
    pub rules: Option<Rules>,
}

impl MergeOptions {
    /// The default: last layer wins, no type checking.
    pub const LAST_WINS: Self = Self {
        strict: false,
        rules: None,
    };
    /// Error when a layer changes the kind of an existing key.
    pub const STRICT: Self = Self {
        strict: true,
        rules: None,
    };
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
    /// A layer supplied a value for a path pinned by [`Strategy::Fail`].
    #[error("`{}` is locked: an earlier layer already set it", render_path(path))]
    Locked { path: Vec<String> },
    /// [`Strategy::Append`] met something other than two arrays.
    #[error("cannot append {found} to {base} at `{}`", render_path(path))]
    AppendKind {
        path: Vec<String>,
        base: &'static str,
        found: &'static str,
    },
}

impl MergeError {
    /// The dotted key path the conflict occurred at.
    pub fn path(&self) -> &[String] {
        match self {
            Self::TypeConflict { path, .. }
            | Self::Locked { path }
            | Self::AppendKind { path, .. } => path,
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
/// Objects recurse per key. Arrays, scalars, datetimes and null all replace
/// wholesale — notably arrays are never index-merged or concatenated, and null
/// is an ordinary value that overwrites rather than a delete instruction.
///
/// [`MergeOptions::rules`] overrides that at the paths it names, and only there.
pub fn merge_into(base: &mut Value, over: Value, opts: &MergeOptions) -> Result<(), MergeError> {
    let mut path = Vec::new();
    apply(base, over, opts, &mut path, opts.rules.as_ref())
}

/// Folds a list of layers into one document, last-wins, seeded with an empty object.
///
/// Equivalent to [`merge_with`] using [`MergeOptions::LAST_WINS`].
pub fn merge(layers: impl IntoIterator<Item = Value>) -> Result<Value, MergeError> {
    merge_with(layers, &MergeOptions::LAST_WINS)
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
///
/// [`Strategy::Append`] does not reintroduce the problem: concatenation is
/// associative, so strict mode still buys associativity with rules in play.
pub fn merge_with(
    layers: impl IntoIterator<Item = Value>,
    opts: &MergeOptions,
) -> Result<Value, MergeError> {
    let mut acc = Value::Object(Map::new());
    let mut path = Vec::new();
    for layer in layers {
        apply(&mut acc, layer, opts, &mut path, opts.rules.as_ref())?;
        debug_assert!(path.is_empty(), "breadcrumb leaked between layers");
    }
    Ok(acc)
}

/// Dispatches one node to its strategy. `rules` is the subtree of rules rooted
/// at `path`, so the lookup is one `BTreeMap` probe per level and `None`
/// short-circuits everything below it.
///
/// Reached only where `base` already holds a value: a key the accumulator does
/// not have yet is inserted without consulting any strategy, which is what
/// keeps [`Fail`](Strategy::Fail) meaning "the first layer to define this pins
/// it" and keeps [`Append`](Strategy::Append) from doubling a lone layer's
/// array against the empty seed.
fn apply(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &mut Vec<String>,
    rules: Option<&Rules>,
) -> Result<(), MergeError> {
    match rules.and_then(Rules::strategy).unwrap_or(Strategy::Merge) {
        Strategy::Merge => merge_at(base, over, opts, path, rules),
        Strategy::Replace => replace(base, over, opts, path),
        Strategy::Append => append(base, over, path),
        Strategy::Fail => Err(MergeError::Locked { path: path.clone() }),
    }
}

/// The recursive worker. `path` is a breadcrumb threaded by push/pop so that a
/// conflict can report where it happened without every frame allocating;
/// `rules` narrows on the same descent.
fn merge_at(
    base: &mut Value,
    over: Value,
    opts: &MergeOptions,
    path: &mut Vec<String>,
    rules: Option<&Rules>,
) -> Result<(), MergeError> {
    match (base, over) {
        (Value::Object(base_map), Value::Object(over_map)) => {
            for (k, v) in over_map {
                let child = rules.and_then(|r| r.child(&k));
                if let Some(slot) = base_map.get_mut(&k) {
                    path.push(k);
                    apply(slot, v, opts, path, child)?;
                    path.pop();
                } else {
                    // No collision, so no strategy applies.
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

/// Concatenates base ++ overlay. The one place a layer adds to a value instead
/// of replacing it, so both sides must really be arrays.
fn append(base: &mut Value, over: Value, path: &[String]) -> Result<(), MergeError> {
    match (base, over) {
        (Value::Array(base_items), Value::Array(over_items)) => {
            base_items.extend(over_items);
            Ok(())
        }
        (base, over) => Err(MergeError::AppendKind {
            path: path.to_vec(),
            base: base.kind(),
            found: over.kind(),
        }),
    }
}
