//! Generic layered merge over native configuration values.

use crate::glob::KeyGlobPattern;
use crate::path::render_keys;
use crate::{ConfigObject, ConfigValue};

/// Knobs on the merge itself. Passed by reference rather than encoded as cargo
/// features: features are additive and unify across a dependency graph, so a
/// `strict` feature would silently change behaviour for one consumer the moment
/// a second consumer enabled it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Error when a layer changes the kind of an existing key.
    pub strict: bool,
    /// Full key paths selected for wholesale replacement, rather than recursion.
    /// `None` is deep merge; `*` replaces top-level values; `foo.*` replaces
    /// the immediate children of `foo`. Matching ancestors stop traversal.
    pub shallow: Option<KeyGlobPattern>,
}

impl MergeOptions {
    /// The default: deep merge, last layer wins, no type checking.
    pub const LAST_WINS: Self = Self {
        strict: false,
        shallow: None,
    };
    /// Error when a layer changes the kind of an existing key.
    pub const STRICT: Self = Self {
        strict: true,
        shallow: None,
    };

    /// Top-level keys only: a later layer's value replaces the earlier one whole.
    pub fn shallow_root() -> Self {
        Self {
            strict: false,
            shallow: Some("*".parse().expect("valid root selector")),
        }
    }
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
/// Colliding values selected by [`MergeOptions::shallow`] replace wholesale,
/// without visiting descendants. `*` makes the root merge shallow (jq's
/// `a + b`); `foo.*` makes only the object at `foo` shallow.
pub fn merge_into<V: ConfigValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_at(base, over, opts, &mut path)
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
/// Flatten first, fold second. (A merge shallow at the root happens to be
/// associative, but the fold does not rely on it.)
pub fn merge<V: ConfigValue>(
    layers: impl IntoIterator<Item = V>,
    opts: &MergeOptions,
) -> Result<V, MergeError> {
    let mut acc = V::object(V::Object::new());
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
/// The breadcrumb names each colliding value before matching the selector.
fn merge_at<V: ConfigValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    if let Some(base_map) = base.as_object_mut() {
        match over.into_object() {
            Ok(over_map) => {
                for (k, v) in over_map {
                    if let Some(slot) = base_map.get_mut(&k) {
                        path.push(k);
                        if opts
                            .shallow
                            .as_ref()
                            .is_some_and(|glob| glob.matches_keys(path))
                        {
                            replace(slot, v, opts, path)?;
                        } else {
                            merge_at(slot, v, opts, path)?;
                        }
                        path.pop();
                    } else {
                        base_map.insert(k, v);
                    }
                }
                return Ok(());
            }
            Err(over) => return replace(base, over, opts, path),
        }
    }
    replace(base, over, opts, path)
}

fn replace<V: ConfigValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
    path: &[String],
) -> Result<(), MergeError> {
    if opts.strict {
        check_kind(base.kind(), over.kind(), path)?;
    }
    *base = over;
    Ok(())
}

/// Errors if a replacement would change the kind of the existing value.
///
/// Strict mode catches the class of mistake where a leaf accidentally shadows a
/// subtree. Pleasant side effect: it rejects exactly the type changes that break
/// associativity, so under strict mode the merge *is* associative.
fn check_kind(
    expected: &'static str,
    found: &'static str,
    path: &[String],
) -> Result<(), MergeError> {
    if expected == found {
        return Ok(());
    }
    Err(MergeError::TypeConflict {
        path: path.to_vec(),
        expected,
        found,
    })
}
