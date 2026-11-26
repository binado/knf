//! `key.path=value` inline configs.

use std::fmt;
use std::str::FromStr;

use crate::{KeyPath, PathError, Value};

/// A leaf value addressed by a parsed key path: `server.port=8080`.
///
/// [`FromStr`] for `PathLeaf<String>` keeps the RHS raw for format-specific parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf<V> {
    path: KeyPath,
    leaf: V,
}

impl<V> PathLeaf<V> {
    /// Build from keys and a leaf. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>, leaf: V) -> Result<Self, PathError> {
        Ok(Self {
            path: KeyPath::from_keys(path)?,
            leaf,
        })
    }

    /// Where the leaf goes.
    pub fn path(&self) -> &KeyPath {
        &self.path
    }

    /// The RHS value, not yet wrapped in nested objects.
    pub fn leaf(&self) -> &V {
        &self.leaf
    }

    /// Replace the leaf, keeping the path.
    pub fn map_leaf<T>(self, f: impl FnOnce(V) -> T) -> PathLeaf<T> {
        PathLeaf {
            path: self.path,
            leaf: f(self.leaf),
        }
    }

    /// [`map_leaf`](Self::map_leaf) when the conversion can fail.
    pub fn try_map_leaf<T, E>(self, f: impl FnOnce(V) -> Result<T, E>) -> Result<PathLeaf<T>, E> {
        Ok(PathLeaf {
            path: self.path,
            leaf: f(self.leaf)?,
        })
    }
}

impl FromStr for PathLeaf<String> {
    type Err = PathError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        // Split on the first `=` so the RHS may contain more of them.
        let Some((lhs, rhs)) = expr.split_once('=') else {
            return Err(PathError::MissingEquals);
        };
        Ok(Self {
            path: lhs.parse()?,
            leaf: rhs.to_string(),
        })
    }
}

impl fmt::Display for PathLeaf<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.path, self.leaf)
    }
}

impl From<PathLeaf<Value>> for Value {
    /// Expands to a nested object.
    fn from(path_leaf: PathLeaf<Value>) -> Self {
        let keys = path_leaf.path.into_keys();
        keys.into_iter().rev().fold(path_leaf.leaf, |value, key| {
            Value::Object([(key, value)].into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RefPathError;

    #[test]
    fn rejects_malformed_expressions() {
        for (bad, want) in [
            ("noequals", PathError::MissingEquals),
            ("=1", PathError::Ref(RefPathError::EmptyPath)),
            (
                "a..b=1",
                PathError::Ref(RefPathError::EmptySegment {
                    path: "a..b".into(),
                }),
            ),
            (
                ".a=1",
                PathError::Ref(RefPathError::EmptySegment { path: ".a".into() }),
            ),
            (
                "a.=1",
                PathError::Ref(RefPathError::EmptySegment { path: "a.".into() }),
            ),
            (
                "a[]=1",
                PathError::Ref(RefPathError::BadIndex { path: "a[]".into() }),
            ),
        ] {
            assert_eq!(bad.parse::<PathLeaf<String>>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::<String>::new(vec![], "1".into()).unwrap_err(),
            PathError::Ref(RefPathError::EmptyPath)
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["".into()], "1".into()).unwrap_err(),
            PathError::Ref(RefPathError::EmptySegment { path: "".into() })
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["a".into(), "".into()], "1".into()).unwrap_err(),
            PathError::Ref(RefPathError::EmptySegment { path: "a.".into() })
        );
    }

    #[test]
    fn raw_fromstr_keeps_the_rhs_unparsed() {
        let path_leaf: PathLeaf<String> = "port=8080".parse().unwrap();
        assert_eq!(path_leaf.path().keys(), ["port"]);
        assert_eq!(path_leaf.leaf(), "8080");
        assert_eq!(path_leaf.to_string(), "port=8080");
    }

    #[test]
    fn path_leaf_rejects_index_steps() {
        assert_eq!(
            "a[0]=1".parse::<PathLeaf<String>>().unwrap_err(),
            PathError::IndexInKeyPath {
                path: "a[0]".into()
            }
        );
    }

    #[test]
    fn map_leaf_preserves_the_path() {
        let path_leaf = PathLeaf::new(vec!["a".into()], "xy".to_string())
            .unwrap()
            .map_leaf(|s| s.len());
        assert_eq!(path_leaf.path().keys(), ["a"]);
        assert_eq!(*path_leaf.leaf(), 2);
    }
}
