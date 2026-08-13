//! A leaf value addressed by a dotted path.
//!
//! [`PathLeaf`] parses `key.path=value`, serde-(de)serializes as that
//! expression string, and expands to a nested [`serde_json::Value`].
//!
//! This crate knows nothing about files or the command line; provenance
//! (`--set`, filenames) is the caller's job.
//!
//! Two conversions, deliberately not the same:
//!
//! - [`From<PathLeaf> for Value`](From) expands to a nested object:
//!   `server.port=8080` → `{"server":{"port":8080}}`.
//! - Serde (de)serializes the expression string: `"server.port=8080"`.
//!   [`serde_json::to_value`] therefore yields a JSON string, not the object.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// A leaf value addressed by a dotted path.
///
/// The RHS of `key.path=value` is parsed as JSON, falling back to a string
/// when that fails: `port=8080` is a number, `name=foo` is a string.
///
/// `Display` is canonical — dotted path, `=`, compact JSON of the leaf — so
/// `name=foo` displays as `name="foo"`. [`FromStr`] ∘ [`Display`] preserves
/// path and leaf, not the original spelling.
#[derive(Debug, Clone, PartialEq)]
pub struct PathLeaf {
    path: Vec<String>,
    leaf: Value,
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

impl PathLeaf {
    /// Build from path segments and a leaf. Rejects an empty path or any empty segment.
    pub fn new(path: Vec<String>, leaf: Value) -> Result<Self, ParseError> {
        if path.is_empty() {
            return Err(ParseError::EmptyKey);
        }
        if path.iter().any(|k| k.is_empty()) {
            return Err(ParseError::EmptySegment {
                key: path.join("."),
            });
        }
        Ok(Self { path, leaf })
    }

    /// Path segments, in order. `server.port` is `["server", "port"]`.
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// The RHS value, not yet wrapped in nested objects.
    pub fn leaf(&self) -> &Value {
        &self.leaf
    }
}

impl FromStr for PathLeaf {
    type Err = ParseError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        // Split on the first `=` so the RHS may contain more of them.
        let Some((lhs, rhs)) = expr.split_once('=') else {
            return Err(ParseError::MissingEquals);
        };
        if lhs.is_empty() {
            return Err(ParseError::EmptyKey);
        }
        let path: Vec<String> = lhs.split('.').map(str::to_string).collect();
        // JSON first, string as the fallback. `port=8080` is a number, `name=foo`
        // is a string because it is not valid JSON, and `tags=[a,b]` is the string
        // "[a,b]" for the same reason.
        let leaf = serde_json::from_str(rhs).unwrap_or_else(|_| Value::String(rhs.to_string()));
        Self::new(path, leaf)
    }
}

impl fmt::Display for PathLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Value` always serializes; a failure here is a serde_json bug.
        let rhs = serde_json::to_string(&self.leaf).expect("Value is always serializable");
        write!(f, "{}={rhs}", self.path.join("."))
    }
}

impl From<PathLeaf> for Value {
    fn from(path_leaf: PathLeaf) -> Self {
        path_leaf
            .path
            .into_iter()
            .rev()
            .fold(path_leaf.leaf, |acc, key| {
                let mut obj = Map::new();
                obj.insert(key, acc);
                Value::Object(obj)
            })
    }
}

impl Serialize for PathLeaf {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PathLeaf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(expr: &str) -> PathLeaf {
        expr.parse().expect("valid PathLeaf")
    }

    fn nested(expr: &str) -> Value {
        Value::from(parse(expr))
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
    fn rejects_malformed_expressions() {
        for (bad, want) in [
            ("noequals", ParseError::MissingEquals),
            ("=1", ParseError::EmptyKey),
            ("a..b=1", ParseError::EmptySegment { key: "a..b".into() }),
            (".a=1", ParseError::EmptySegment { key: ".a".into() }),
            ("a.=1", ParseError::EmptySegment { key: "a.".into() }),
        ] {
            assert_eq!(bad.parse::<PathLeaf>().unwrap_err(), want, "{bad}");
        }
    }

    #[test]
    fn new_rejects_empty_path_and_empty_segments() {
        assert_eq!(
            PathLeaf::new(vec![], json!(1)).unwrap_err(),
            ParseError::EmptyKey
        );
        assert_eq!(
            PathLeaf::new(vec!["".into()], json!(1)).unwrap_err(),
            ParseError::EmptySegment { key: "".into() }
        );
        assert_eq!(
            PathLeaf::new(vec!["a".into(), "".into()], json!(1)).unwrap_err(),
            ParseError::EmptySegment { key: "a.".into() }
        );
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
            let round = parsed.to_string().parse::<PathLeaf>().unwrap();
            assert_eq!(round.path(), parsed.path(), "{expr}");
            assert_eq!(round.leaf(), parsed.leaf(), "{expr}");
        }
    }

    #[test]
    fn serde_is_the_expression_string_not_the_object() {
        let path_leaf = parse("port=8080");
        assert_eq!(
            serde_json::to_value(&path_leaf).unwrap(),
            json!("port=8080")
        );
        assert_eq!(Value::from(path_leaf.clone()), json!({"port": 8080}));

        let path_leaf = parse("name=foo");
        assert_eq!(
            serde_json::to_value(&path_leaf).unwrap(),
            json!(r#"name="foo""#)
        );

        let back: PathLeaf = serde_json::from_value(json!("server.port=8080")).unwrap();
        assert_eq!(back.path(), ["server", "port"]);
        assert_eq!(back.leaf(), &json!(8080));
    }
}
