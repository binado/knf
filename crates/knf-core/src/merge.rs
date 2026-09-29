//! Generic layered merge over native configuration values.

use crate::glob::KeyGlobPattern;
use crate::path::render_keys;
use crate::{ConfigObject, ConfigValue, Seg};

/// Inheritance uses full key segments too; array descendants have no selector.
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

/// Ordered object overlay shared by native layers and inherited projections.
pub(crate) fn merge_fields<V, O: ConfigObject<V>, E>(
    base: &mut O,
    over: O,
    mut merge: impl FnMut(&mut V, V, String) -> Result<(), E>,
) -> Result<(), E> {
    for (key, value) in over {
        if let Some(slot) = base.get_mut(&key) {
            merge(slot, value, key)?;
        } else {
            base.insert(key, value);
        }
    }
    Ok(())
}

/// Merge options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Error when a layer changes the kind of an existing key.
    pub strict: bool,
    /// Key paths to replace wholesale: `*` (top level), `foo`, `foo.*`.
    /// `None` is a deep merge.
    pub shallow: Option<KeyGlobPattern>,
}

impl MergeOptions {
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

/// Merges `over` into `base` in place (jq's `a * b`).
///
/// Objects recurse per key; everything else, including arrays and null,
/// replaces. Paths matched by [`MergeOptions::shallow`] replace wholesale.
pub fn merge_into<V: ConfigValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
) -> Result<(), MergeError> {
    let mut path = Vec::new();
    merge_at(base, over, opts, &mut path)
}

/// Left-folds layers into one document, starting from an empty object.
///
/// The merge is not associative, so never merge subgroups and combine them:
///
/// ```text
/// {a:{b:1}} * {a:5} * {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
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

/// The recursive worker. `path` is the current key path, for errors and
/// selector matching.
fn merge_at<V: ConfigValue>(
    base: &mut V,
    over: V,
    opts: &MergeOptions,
    path: &mut Vec<String>,
) -> Result<(), MergeError> {
    if let Some(base_map) = base.as_object_mut() {
        match over.into_object() {
            Ok(over_map) => {
                merge_fields(base_map, over_map, |slot, v, key| {
                    path.push(key);
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
                    Ok(())
                })?;
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
