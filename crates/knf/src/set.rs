//! `--set key.path=value` — a terminal layer built from the command line.
//!
//! Each `--set` becomes its own layer appended after all files and folded
//! through the same `merge_all`, so it introduces no new merge semantics and
//! `--strict` applies to it as well.

use anyhow::{Context, bail};
use serde_json::{Map, Value};

/// Turns `key.path=value` into a nested `Value`.
pub fn parse(expr: &str) -> anyhow::Result<Value> {
    // Split on the *first* `=` so the RHS may contain more of them, e.g.
    // `--set query='a=b'`.
    let (lhs, rhs) = expr
        .split_once('=')
        .with_context(|| format!("`--set {expr}`: expected KEY.PATH=VALUE"))?;

    if lhs.is_empty() {
        bail!("`--set {expr}`: empty key");
    }
    let keys: Vec<&str> = lhs.split('.').collect();
    if keys.iter().any(|k| k.is_empty()) {
        bail!("`--set {expr}`: empty segment in key `{lhs}`");
    }

    // JSON first, string as the fallback. `port=8080` is a number, `name=foo`
    // is a string because it is not valid JSON, and `tags=[a,b]` is the string
    // "[a,b]" for the same reason.
    let leaf = serde_json::from_str(rhs).unwrap_or_else(|_| Value::String(rhs.to_string()));

    Ok(keys.iter().rev().fold(leaf, |acc, key| {
        let mut obj = Map::new();
        obj.insert((*key).to_string(), acc);
        Value::Object(obj)
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn set(expr: &str) -> serde_json::Value {
        super::parse(expr).unwrap()
    }

    /// The §4.2 table, verbatim.
    #[test]
    fn value_typing() {
        assert_eq!(set("port=8080"), json!({"port": 8080}));
        assert_eq!(set("debug=true"), json!({"debug": true}));
        assert_eq!(set("name=foo"), json!({"name": "foo"}));
        assert_eq!(set("proxy=null"), json!({"proxy": null}));
        assert_eq!(set(r#"tags=["a","b"]"#), json!({"tags": ["a", "b"]}));
        assert_eq!(set("tags=[a,b]"), json!({"tags": "[a,b]"}));
    }

    /// The sharp edge documented in `--help`: a bare `1.0` is a number.
    #[test]
    fn numeric_looking_strings() {
        assert_eq!(set("version=1.0"), json!({"version": 1.0}));
        assert_eq!(set(r#"version="1.0""#), json!({"version": "1.0"}));
    }

    #[test]
    fn dotted_paths_nest() {
        assert_eq!(set("server.port=8080"), json!({"server": {"port": 8080}}));
        assert_eq!(set("a.b.c=1"), json!({"a": {"b": {"c": 1}}}));
    }

    #[test]
    fn splits_on_the_first_equals_only() {
        assert_eq!(set("q=a=b"), json!({"q": "a=b"}));
        assert_eq!(set("q="), json!({"q": ""}));
    }

    #[test]
    fn rejects_malformed_expressions() {
        for bad in ["noequals", "=1", "a..b=1", ".a=1", "a.=1"] {
            assert!(
                super::parse(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }
}
