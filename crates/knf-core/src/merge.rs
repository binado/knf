//! Layered configuration merge.

use crate::glob::KeyGlobPattern;
use crate::{InterpError, InterpolationOptions, Seg, Value, merge_with_interpolation};

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
    /// Top-level keys only: a later config's value replaces the earlier one whole.
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

/// Left-folds configs into one document, starting from an empty object.
///
/// Merging is not associative (a scalar config shadows earlier objects):
///
/// ```text
/// {a:{b:1}} * {a:5} * {a:{c:2}}
///   left-assoc  -> {a:{c:2}}
///   right-assoc -> {a:{b:1,c:2}}
/// ```
pub fn merge(configs: impl IntoIterator<Item = Value>, opts: &MergeOptions) -> Value {
    let mut acc = Value::Object(Default::default());
    let mut path = Vec::new();
    for config in configs {
        merge_at(&mut acc, config, opts, &mut path);
        debug_assert!(path.is_empty(), "breadcrumb leaked between configs");
    }
    acc
}

/// Left-folds configs into one document, resolving references when
/// `interpolation` is given.
///
/// `None` is [`merge`] and cannot fail; `Some` is [`merge_with_interpolation`].
pub fn merge_configs(
    configs: impl IntoIterator<Item = Value>,
    merge_options: &MergeOptions,
    interpolation: Option<InterpolationOptions>,
) -> Result<Value, InterpError> {
    match interpolation {
        Some(options) => merge_with_interpolation(configs, merge_options, &options),
        None => Ok(merge(configs, merge_options)),
    }
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

#[deprecated(note = "use merge_configs")]
pub fn merge_layers(
    configs: impl IntoIterator<Item = Value>,
    merge_options: &MergeOptions,
    interpolation: Option<InterpolationOptions>,
) -> Result<Value, InterpError> {
    merge_configs(configs, merge_options, interpolation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(entries: &[(&str, &str)]) -> Value {
        Value::Object(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), Value::String((*value).to_owned())))
                .collect(),
        )
    }

    fn configs() -> [Value; 2] {
        [object(&[("a", "1")]), object(&[("b", "${a}")])]
    }

    #[test]
    fn merge_configs_resolves_references_only_with_interpolation() {
        let opts = MergeOptions::default();
        assert_eq!(
            merge_configs(configs(), &opts, None).unwrap(),
            object(&[("a", "1"), ("b", "${a}")])
        );
        let interpolation = InterpolationOptions::new(&Value::String);
        assert_eq!(
            merge_configs(configs(), &opts, Some(interpolation)).unwrap(),
            object(&[("a", "1"), ("b", "1")])
        );
    }
}
