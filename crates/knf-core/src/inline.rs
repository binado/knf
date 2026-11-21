//! `key.path=value` inline layers.

use std::fmt;
use std::str::FromStr;

use crate::{PathError, RefPath, Seg, Value};

/// A leaf value addressed by a parsed path: `server.port=8080`.
///
/// [`FromStr`] for `PathLeaf<String>` keeps the RHS raw for format-specific parsing.
/// Bracket steps parse, but conversion to a layer rejects them.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf<V> {
    path: RefPath,
    leaf: V,
}

impl<V> PathLeaf<V> {
    /// Build from path segments and a leaf. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>, leaf: V) -> Result<Self, PathError> {
        Ok(Self {
            path: RefPath::from_keys(path)?,
            leaf,
        })
    }

    /// The path as steps; may include index steps.
    pub fn path(&self) -> &[Seg] {
        self.path.segs()
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

impl PathLeaf<String> {
    /// Check that the path contains keys only.
    pub fn validate_keys(&self) -> Result<(), PathError> {
        self.path.clone().try_into_keys().map(|_| ())
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

impl TryFrom<PathLeaf<Value>> for Value {
    type Error = PathError;

    /// Expands to a nested object. Fails on index steps.
    fn try_from(path_leaf: PathLeaf<Value>) -> Result<Self, Self::Error> {
        let keys = path_leaf.path.try_into_keys()?;
        Ok(keys.into_iter().rev().fold(path_leaf.leaf, |value, key| {
            Value::Object([(key, value)].into())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_expressions() {
        for (bad, want) in [
            ("noequals", PathError::MissingEquals),
            ("=1", PathError::EmptyPath),
            (
                "a..b=1",
                PathError::EmptySegment {
                    path: "a..b".into(),
                },
            ),
            (".a=1", PathError::EmptySegment { path: ".a".into() }),
            ("a.=1", PathError::EmptySegment { path: "a.".into() }),
            ("a[]=1", PathError::BadIndex { path: "a[]".into() }),
        ] {
            assert_eq!(bad.parse::<PathLeaf<String>>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::<String>::new(vec![], "1".into()).unwrap_err(),
            PathError::EmptyPath
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["".into()], "1".into()).unwrap_err(),
            PathError::EmptySegment { path: "".into() }
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["a".into(), "".into()], "1".into()).unwrap_err(),
            PathError::EmptySegment { path: "a.".into() }
        );
    }

    #[test]
    fn raw_fromstr_keeps_the_rhs_unparsed() {
        let path_leaf: PathLeaf<String> = "port=8080".parse().unwrap();
        assert_eq!(path_leaf.path(), [Seg::Key("port".into())]);
        assert_eq!(path_leaf.leaf(), "8080");
        assert_eq!(path_leaf.to_string(), "port=8080");
    }

    #[test]
    fn path_leaf_accepts_brackets_its_writers_reject() {
        let parsed: PathLeaf<String> = "a[0]=1".parse().unwrap();
        assert_eq!(parsed.path(), [Seg::Key("a".into()), Seg::Index(0)]);
        assert_eq!(parsed.to_string(), "a[0]=1");
    }

    #[test]
    fn map_leaf_preserves_the_path() {
        let path_leaf = PathLeaf::new(vec!["a".into()], "xy".to_string())
            .unwrap()
            .map_leaf(|s| s.len());
        assert_eq!(path_leaf.path(), [Seg::Key("a".into())]);
        assert_eq!(*path_leaf.leaf(), 2);
    }
}
