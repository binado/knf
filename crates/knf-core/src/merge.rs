//! Generic layered merge over native configuration values.

use crate::glob::KeyGlobPattern;
use crate::path::render_keys;
use crate::{ConfigObject, ConfigValue};

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
