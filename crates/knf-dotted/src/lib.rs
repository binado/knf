//! A leaf value addressed by a dotted path.
//!
//! [`PathLeaf`] parses `key.path=value`. The path is always typed; the leaf
//! type `V` is chosen by the caller. [`FromStr`] for [`PathLeaf<String>`] keeps
//! the RHS raw. The `json` feature parses that RHS as JSON into
//! [`serde_json::Value`], falling back to a string.
//!
//! [`KeyPath`] is the same path with no `=value` half, for callers that address
//! a location rather than assign to one.
//!
//! This crate knows nothing about files or the command line; provenance
//! (`--set`, filenames) is the caller's job.
//!
//! [`From<PathLeaf<V>>`](From) expands to a nested object: `server.port=8080` →
//! `{"server":{"port":8080}}`. There are deliberately no `Serialize`/
//! `Deserialize` impls — a `PathLeaf` is an expression, and serializing one
//! could reasonably mean either the string or the object, so callers pick
//! explicitly via [`Display`](fmt::Display) or the conversion.

use std::fmt;
use std::str::FromStr;

/// A leaf value addressed by a dotted path.
///
/// [`FromStr`] for [`PathLeaf<String>`] splits `key.path=value` and stores the
/// RHS as-is. The typed [`FromStr`] impl (behind `json`) parses that RHS as
/// JSON, falling back to a string: `port=8080` is a number, `name=foo` is a
/// string.
///
/// `Display` of a typed leaf is canonical — dotted path, `=`, compact JSON of
/// the leaf — so `name=foo` displays as `name="foo"`. [`FromStr`] ∘ [`Display`](fmt::Display)
/// preserves path and leaf, not the original spelling. [`PathLeaf<String>`]
/// displays the raw RHS.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf<V> {
    path: KeyPath,
    leaf: V,
}

/// Why a `key.path=value` expression (or a programmatic constructor) was rejected.
///
/// Carries the dotted key when a segment is empty, and nothing else — no
/// `--set`, no filenames. Provenance is the caller's job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// No `=` in the expression.
    #[error("expected KEY.PATH=VALUE")]
    MissingEquals,
    /// The path is empty (`=1`, or `new` with an empty vec).
    #[error("empty key")]
    EmptyKey,
    /// A path segment is empty (`a..b=1`, `.a=1`, `a.=1`).
    #[error("empty segment in key `{key}`")]
    EmptySegment {
        /// The dotted path that contained an empty segment.
        key: String,
    },
}

/// A validated dotted path, with no `=value` half.
///
/// The same path [`PathLeaf`] carries, for callers that address a location
/// rather than assign to one. [`PathLeaf::new`] is defined in terms of it, so
/// the empty-key and empty-segment rules exist in exactly one place.
///
/// [`ParseError::MissingEquals`] is shared but unreachable from
/// [`FromStr::from_str`]: there is no `=` to miss.
///
/// Dots separate segments, so a key containing a literal dot is not addressable
/// by a `KeyPath`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyPath {
    path: Vec<String>,
}

impl KeyPath {
    /// Build from segments. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>) -> Result<Self, ParseError> {
        if path.is_empty() {
            return Err(ParseError::EmptyKey);
        }
        if path.iter().any(|k| k.is_empty()) {
            return Err(ParseError::EmptySegment {
                key: path.join("."),
            });
        }
        Ok(Self { path })
    }

    /// Path segments, in order. `server.port` is `["server", "port"]`.
    pub fn segments(&self) -> &[String] {
        &self.path
    }

    /// The owned segments.
    pub fn into_segments(self) -> Vec<String> {
        self.path
    }
}

impl FromStr for KeyPath {
    type Err = ParseError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Self::new(split_key(key)?)
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path.join("."))
    }
}

impl<V> PathLeaf<V> {
    /// Build from path segments and a leaf. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>, leaf: V) -> Result<Self, ParseError> {
        Ok(Self {
            path: KeyPath::new(path)?,
            leaf,
        })
    }

    /// Path segments, in order. `server.port` is `["server", "port"]`.
    pub fn path(&self) -> &[String] {
        self.path.segments()
    }

    /// The RHS value, not yet wrapped in nested objects.
    pub fn leaf(&self) -> &V {
        &self.leaf
    }

    /// Replace the leaf, keeping the path. The path is already valid, so this
    /// cannot fail the way [`new`](Self::new) can.
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

    #[cfg(feature = "json")]
    fn into_nested(self, nest: impl Fn(String, V) -> V) -> V {
        self.path
            .into_segments()
            .into_iter()
            .rev()
            .fold(self.leaf, |acc, key| nest(key, acc))
    }
}

impl FromStr for PathLeaf<String> {
    type Err = ParseError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        let (path, rhs) = split_expr(expr)?;
        Self::new(path, rhs.to_string())
    }
}

impl fmt::Display for PathLeaf<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.path, self.leaf)
    }
}

fn split_expr(expr: &str) -> Result<(Vec<String>, &str), ParseError> {
    // Split on the first `=` so the RHS may contain more of them.
    let Some((lhs, rhs)) = expr.split_once('=') else {
        return Err(ParseError::MissingEquals);
    };
    Ok((split_key(lhs)?, rhs))
}

/// Splits a dotted key into segments. `""` is an empty key rather than one
/// empty segment, which is the more useful of the two messages.
fn split_key(key: &str) -> Result<Vec<String>, ParseError> {
    if key.is_empty() {
        return Err(ParseError::EmptyKey);
    }
    Ok(key.split('.').map(str::to_string).collect())
}

#[cfg(feature = "json")]
mod json;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_expressions() {
        for (bad, want) in [
            ("noequals", ParseError::MissingEquals),
            ("=1", ParseError::EmptyKey),
            ("a..b=1", ParseError::EmptySegment { key: "a..b".into() }),
            (".a=1", ParseError::EmptySegment { key: ".a".into() }),
            ("a.=1", ParseError::EmptySegment { key: "a.".into() }),
        ] {
            assert_eq!(bad.parse::<PathLeaf<String>>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::<String>::new(vec![], "1".into()).unwrap_err(),
            ParseError::EmptyKey
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["".into()], "1".into()).unwrap_err(),
            ParseError::EmptySegment { key: "".into() }
        );
        assert_eq!(
            PathLeaf::<String>::new(vec!["a".into(), "".into()], "1".into()).unwrap_err(),
            ParseError::EmptySegment { key: "a.".into() }
        );
    }

    #[test]
    fn raw_fromstr_keeps_the_rhs_unparsed() {
        let path_leaf: PathLeaf<String> = "port=8080".parse().unwrap();
        assert_eq!(path_leaf.path(), ["port"]);
        assert_eq!(path_leaf.leaf(), "8080");
        assert_eq!(path_leaf.to_string(), "port=8080");
    }

    /// The rules live in `KeyPath::new`, so both types reject the same spellings.
    #[test]
    fn key_path_rejects_what_path_leaf_rejects() {
        for (bad, want) in [
            ("", ParseError::EmptyKey),
            ("a..b", ParseError::EmptySegment { key: "a..b".into() }),
            (".a", ParseError::EmptySegment { key: ".a".into() }),
            ("a.", ParseError::EmptySegment { key: "a.".into() }),
        ] {
            assert_eq!(bad.parse::<KeyPath>().unwrap_err(), want, "{bad}");
        }
        assert_eq!(KeyPath::new(vec![]).unwrap_err(), ParseError::EmptyKey);
    }

    #[test]
    fn key_path_round_trips_through_display() {
        let key: KeyPath = "db.plugins".parse().unwrap();
        assert_eq!(key.segments(), ["db", "plugins"]);
        assert_eq!(key.to_string(), "db.plugins");
        assert_eq!(key.into_segments(), ["db", "plugins"]);
    }

    /// A `=` has no special meaning without a leaf to assign, so it is just part
    /// of a (weird) key rather than a `MissingEquals`-shaped hole.
    #[test]
    fn key_path_has_no_equals_half() {
        assert_eq!("a=b".parse::<KeyPath>().unwrap().segments(), ["a=b"]);
    }

    #[test]
    fn map_leaf_preserves_the_path() {
        let path_leaf = PathLeaf::new(vec!["a".into()], "xy".to_string())
            .unwrap()
            .map_leaf(|s| s.len());
        assert_eq!(path_leaf.path(), ["a"]);
        assert_eq!(*path_leaf.leaf(), 2);
    }
}
