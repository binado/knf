//! TOML leaf: parse the RHS as JSON into `toml::Value`, expand to nested tables.
//!
//! JSON null has no TOML equivalent, so it falls through to the string `"null"`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{ParseError, PathLeaf, write_canonical};

impl FromStr for PathLeaf<toml::Value> {
    type Err = ParseError;

    fn from_str(expr: &str) -> Result<Self, Self::Err> {
        Ok(PathLeaf::<String>::from_str(expr)?.into())
    }
}

impl From<PathLeaf<String>> for PathLeaf<toml::Value> {
    fn from(path_leaf: PathLeaf<String>) -> Self {
        path_leaf.map_leaf(|rhs| parse_leaf(&rhs))
    }
}

fn parse_leaf(rhs: &str) -> toml::Value {
    serde_json::from_str(rhs).unwrap_or_else(|_| toml::Value::String(rhs.to_string()))
}

impl fmt::Display for PathLeaf<toml::Value> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_canonical(&self.path, &self.leaf, f)
    }
}

impl From<PathLeaf<toml::Value>> for toml::Value {
    fn from(path_leaf: PathLeaf<toml::Value>) -> Self {
        path_leaf.into_nested(|key, acc| {
            let mut table = toml::Table::new();
            table.insert(key, acc);
            toml::Value::Table(table)
        })
    }
}

impl Serialize for PathLeaf<toml::Value> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PathLeaf<toml::Value> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(expr: &str) -> PathLeaf<toml::Value> {
        expr.parse().expect("valid PathLeaf")
    }

    fn nested(expr: &str) -> toml::Value {
        toml::Value::from(parse(expr))
    }

    #[test]
    fn value_typing() {
        assert_eq!(nested("port=8080")["port"].as_integer(), Some(8080));
        assert_eq!(nested("debug=true")["debug"].as_bool(), Some(true));
        assert_eq!(nested("name=foo")["name"].as_str(), Some("foo"));
        assert_eq!(nested("proxy=null")["proxy"].as_str(), Some("null"));
        assert_eq!(
            nested(r#"tags=["a","b"]"#)["tags"]
                .as_array()
                .map(|a| { a.iter().filter_map(toml::Value::as_str).collect::<Vec<_>>() }),
            Some(vec!["a", "b"])
        );
        assert_eq!(nested("tags=[a,b]")["tags"].as_str(), Some("[a,b]"));
    }

    #[test]
    fn dotted_paths_nest() {
        assert_eq!(
            nested("server.port=8080")["server"]["port"].as_integer(),
            Some(8080)
        );
        assert_eq!(nested("a.b.c=1")["a"]["b"]["c"].as_integer(), Some(1));
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
            "server.port=8080",
        ] {
            let parsed = parse(expr);
            let round = parsed.to_string().parse::<PathLeaf<toml::Value>>().unwrap();
            assert_eq!(round.path(), parsed.path(), "{expr}");
            assert_eq!(round.leaf(), parsed.leaf(), "{expr}");
        }
    }

    #[test]
    fn from_raw_path_leaf_parses_the_rhs() {
        let raw: PathLeaf<String> = "server.port=8080".parse().unwrap();
        let typed = PathLeaf::<toml::Value>::from(raw);
        assert_eq!(
            toml::Value::from(typed)["server"]["port"].as_integer(),
            Some(8080)
        );
    }
}
