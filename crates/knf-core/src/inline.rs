//! `key.path=value` inline layers.

use std::fmt;
use std::str::FromStr;

use crate::{ConfigFormat, ConfigObject, PathError, RefPath, Seg};
use serde_json::{Map, Value};

/// A leaf value addressed by a parsed path: `server.port=8080`.
///
/// [`FromStr`] for `PathLeaf<String>` keeps the RHS raw; the
/// [`serde_json::Value`] impl parses it as JSON with a string fallback.
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

    /// Nests the leaf under every key in the path, innermost first.
    fn try_into_nested(self, nest: impl Fn(String, V) -> V) -> Result<V, PathError> {
        let keys = self.path.try_into_keys()?;
        Ok(keys
            .into_iter()
            .rev()
            .fold(self.leaf, |acc, key| nest(key, acc)))
    }
}

impl PathLeaf<String> {
    /// Check that the path contains keys only.
    pub fn validate_keys(&self) -> Result<(), PathError> {
        self.path.clone().try_into_keys().map(|_| ())
    }

    /// Type the RHS in the native format and expand it into a nested object.
    pub fn into_layer<V: ConfigFormat>(self) -> Result<V, PathError> {
        let keys = self.path.try_into_keys()?;
        let leaf = V::parse_inline(self.leaf);
        Ok(keys.into_iter().rev().fold(leaf, |value, key| {
            let mut object = V::Object::new();
            object.insert(key, value);
            V::object(object)
        }))
    }
}

/// Parses text as a TOML value, falling back to the original text as a string.
pub fn toml_or_string(text: String) -> toml::Value {
    text.trim_matches([' ', '\t', '\r', '\n'])
        .parse()
        .unwrap_or_else(|_| toml::Value::String(text))
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

/// Parses text as JSON, falling back to the string itself: `8080` is a number,
/// `foo` is `"foo"`.
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
        let rhs = serde_json::to_string(&self.leaf).expect("a Value always serializes");
        write!(f, "{}={rhs}", self.path)
    }
}

impl TryFrom<PathLeaf<Value>> for Value {
    type Error = PathError;

    /// Expands to a nested object. Fails on index steps.
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
    fn toml_inline_literals_ignore_surrounding_whitespace() {
        for literal in [
            "8080",
            "true",
            "1.0",
            "1979-05-27",
            "inf",
            "[1, 2]",
            "{a=1}",
            "' name '",
        ] {
            for padding in [" ", "\t", "\n", "\r\n", " \t\r\n"] {
                let expected = toml_or_string(literal.into());
                for text in [
                    format!("{padding}{literal}"),
                    format!("{literal}{padding}"),
                    format!("{padding}{literal}{padding}"),
                ] {
                    assert_eq!(toml_or_string(text.clone()), expected, "{text:?}");
                }
            }
        }
        for text in [
            " \tnull\n",
            " 9223372036854775808\n",
            " [a,b] ",
            " text\n",
            " \t\r\n",
            "\u{a0}8080\u{a0}",
        ] {
            assert_eq!(
                toml_or_string(text.into()),
                toml::Value::String(text.into())
            );
        }
    }

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

    /// Bracket steps parse but never expand into a layer.
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
