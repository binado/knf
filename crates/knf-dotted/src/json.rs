//! JSON leaf: parse the RHS as JSON, expand to a nested object.

use std::fmt;
use std::str::FromStr;

use serde_json::{Map, Value};

use crate::{ParseError, PathLeaf};

impl FromStr for PathLeaf<Value> {
    type Err = ParseError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        Ok(PathLeaf::<String>::from_str(expr)?.into())
    }
}

impl From<PathLeaf<String>> for PathLeaf<Value> {
    fn from(path_leaf: PathLeaf<String>) -> Self {
        // JSON first, string as the fallback. `port=8080` is a number, `name=foo`
        // is a string because it is not valid JSON, and `tags=[a,b]` is the string
        // "[a,b]" for the same reason.
        path_leaf.map_leaf(|rhs| serde_json::from_str(&rhs).unwrap_or_else(|_| Value::String(rhs)))
    }
}

impl fmt::Display for PathLeaf<Value> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Infallible for a `Value`: only maps with non-string keys and
        // non-finite floats can fail, and neither survives a JSON parse.
        let rhs = serde_json::to_string(&self.leaf).expect("a Value always serializes");
        write!(f, "{}={rhs}", self.path.join("."))
    }
}

impl From<PathLeaf<Value>> for Value {
    fn from(path_leaf: PathLeaf<Value>) -> Self {
        path_leaf.into_nested(|key, acc| {
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

    fn parse(expr: &str) -> PathLeaf<Value> {
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
        assert_eq!(Value::from(typed), json!({"server": {"port": 8080}}));
    }
}
