//! `key.path=value` expressions: the one inline-layer spelling.
//!
//! [`PathLeaf`] pairs a [`RefPath`] with a leaf value. The path is always
//! typed; the leaf type `V` is chosen by the caller. [`FromStr`] for
//! [`PathLeaf<String>`] keeps the right-hand side raw, and the
//! [`serde_json::Value`] impl parses it as JSON with a string fallback — a rule
//! exported on its own as [`json_or_string`], for callers that need the same
//! typing without a path.
//!
//! This module lives here rather than beside [`RefPath`] in `knf-core` for a
//! reason the compiler enforces: `impl FromStr for PathLeaf<serde_json::Value>`
//! and its siblings name no local type unless `PathLeaf` itself is local, so
//! the JSON impls and the struct cannot be separated. `knf-core` must not gain
//! `serde_json`, which settles which side of the boundary both land on.
//!
//! [`TryFrom<PathLeaf<Value>>`](TryFrom) expands to a nested object:
//! `server.port=8080` → `{"server":{"port":8080}}`. There are deliberately no
//! `Serialize`/`Deserialize` impls — a `PathLeaf` is an expression, and
//! serializing one could reasonably mean either the string or the object, so
//! callers pick explicitly via [`Display`](fmt::Display) or the conversion.

use std::fmt;
use std::str::FromStr;

use knf_core::{PathError, RefPath, Seg};
use serde_json::{Map, Value};

/// A leaf value addressed by a parsed path.
///
/// [`FromStr`] for [`PathLeaf<String>] splits `key.path=value`, parses the LHS
/// as a [`RefPath`], and stores the RHS as-is. The typed [`FromStr`] impl
/// (behind `json`) parses that RHS as JSON, falling back to a string:
/// `port=8080` is a number, `name=foo` is a string.
///
/// The grammar accepts bracket steps — `a[0]=1` parses — because it is the
/// one spelling references also use. Whether such a path may *write* is
/// [`RefPath::try_into_keys`]' question, asked at conversion time.
///
/// `Display` of a typed leaf is canonical — path, `=`, compact JSON of the
/// leaf — so `name=foo` displays as `name="foo"`. [`FromStr`] ∘ [`Display`](fmt::Display)
/// preserves path and leaf, not the original spelling. [`PathLeaf<String>`]
/// displays the raw RHS.
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

    /// The path as steps. May hold [`Index`](Seg::Index) steps from the
    /// grammar — consumers that write run
    /// [`try_into_keys`](RefPath::try_into_keys).
    pub fn path(&self) -> &[Seg] {
        self.path.segs()
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

    /// Nests the leaf under every key in the path, innermost first.
    fn try_into_nested(self, nest: impl Fn(String, V) -> V) -> Result<V, PathError> {
        let keys = self.path.try_into_keys()?;
        Ok(keys
            .into_iter()
            .rev()
            .fold(self.leaf, |acc, key| nest(key, acc)))
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
impl FromStr for PathLeaf<Value> {
    type Err = PathError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        Ok(PathLeaf::<String>::from_str(expr)?.into())
    }
}

/// Parses text as JSON, falling back to the string itself.
///
/// `8080` is a number, `true` is a bool, `foo` is the string `"foo"` because it
/// is not valid JSON, and `[a,b]` is the string `"[a,b]"` for the same reason.
///
/// Public because more than one caller needs *this* rule rather than a rule like
/// it: `--set`'s RHS and `${env:VAR}` in a whole-string position must type
/// identically, and two matching implementations would only agree until one of
/// them was edited.
pub fn json_or_string(text: String) -> Value {
    serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text))
}

impl From<PathLeaf<String>> for PathLeaf<Value> {
    fn from(path_leaf: PathLeaf<String>) -> Self {
        path_leaf.map_leaf(json_or_string)
    }
}

impl fmt::Display for PathLeaf<Value> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Infallible for a `Value`: only maps with non-string keys and
        // non-finite floats can fail, and neither survives a JSON parse.
        let rhs = serde_json::to_string(&self.leaf).expect("a Value always serializes");
        write!(f, "{}={rhs}", self.path)
    }
}

impl TryFrom<PathLeaf<Value>> for Value {
    type Error = PathError;

    /// Expands to a nested object. Fallible because the grammar accepts
    /// bracket steps (`a[0]=1` parses) that a writer cannot use: an index
    /// never reaches the nested-object expansion.
    fn try_from(path_leaf: PathLeaf<Value>) -> Result<Self, Self::Error> {
        path_leaf.try_into_nested(|key, acc| {
            let mut obj = Map::new();
            obj.insert(key, acc);
            Value::Object(obj)
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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

    /// The grammar accepts bracket steps; only writers reject them.
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

    fn parse(expr: &str) -> PathLeaf<Value> {
        expr.parse().expect("valid PathLeaf")
    }

    fn nested(expr: &str) -> Value {
        Value::try_from(parse(expr)).expect("all-key path")
    }

    /// The §4.2 table, verbatim.
    #[test]
    fn value_typing() {
        assert_eq!(nested("port=8080"), json!({"port": 8080}));
        assert_eq!(nested("debug=true"), json!({"debug": true}));
        assert_eq!(nested("name=foo"), json!({"name": "foo"}));
        assert_eq!(nested("proxy=null"), json!({"proxy": null}));
        assert_eq!(nested(r#"tags=["a","b"]"#), json!({"tags": ["a", "b"]}));
        assert_eq!(nested("tags=[a,b]"), json!({"tags": "[a,b]"}));
    }

    /// The sharp edge: a bare `1.0` is a number.
    #[test]
    fn numeric_looking_strings() {
        assert_eq!(nested("version=1.0"), json!({"version": 1.0}));
        assert_eq!(nested(r#"version="1.0""#), json!({"version": "1.0"}));
    }

    #[test]
    fn dotted_paths_nest() {
        assert_eq!(
            nested("server.port=8080"),
            json!({"server": {"port": 8080}})
        );
        assert_eq!(nested("a.b.c=1"), json!({"a": {"b": {"c": 1}}}));
    }

    #[test]
    fn splits_on_the_first_equals_only() {
        assert_eq!(nested("q=a=b"), json!({"q": "a=b"}));
        assert_eq!(nested("q="), json!({"q": ""}));
    }

    #[test]
    fn display_is_canonical() {
        assert_eq!(parse("name=foo").to_string(), r#"name="foo""#);
        assert_eq!(parse("port=8080").to_string(), "port=8080");
        assert_eq!(parse("q=").to_string(), r#"q="""#);
        assert_eq!(parse("q=a=b").to_string(), r#"q="a=b""#);
        assert_eq!(parse("server.port=8080").to_string(), "server.port=8080");
    }

    #[test]
    fn fromstr_display_preserves_path_and_leaf() {
        for expr in [
            "port=8080",
            "name=foo",
            r#"name="foo""#,
            "debug=true",
            "proxy=null",
            r#"tags=["a","b"]"#,
            "q=",
            "q=a=b",
            "server.port=8080",
        ] {
            let parsed = parse(expr);
            let round = parsed.to_string().parse::<PathLeaf<Value>>().unwrap();
            assert_eq!(round.path(), parsed.path(), "{expr}");
            assert_eq!(round.leaf(), parsed.leaf(), "{expr}");
        }
    }

    #[test]
    fn from_raw_path_leaf_parses_the_rhs() {
        let raw: PathLeaf<String> = "server.port=8080".parse().unwrap();
        let typed = PathLeaf::<Value>::from(raw);
        assert_eq!(
            Value::try_from(typed).unwrap(),
            json!({"server": {"port": 8080}})
        );
    }

    /// Brackets parse — a reference may read an element — but a `--set`-shaped
    /// expression can never expand one into a writer's nested object.
    #[test]
    fn bracketed_paths_parse_but_cannot_write() {
        let err = Value::try_from(parse("servers[0].host=x")).unwrap_err();
        assert_eq!(
            err,
            PathError::IndexInKeyPath {
                path: "servers[0].host".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "`servers[0].host` contains an array index; merge paths take keys only"
        );
    }
}
