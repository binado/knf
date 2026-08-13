//! Native JSON and TOML documents, and the explicit conversion between them.
//!
//! Merge runs on the inner `serde_json::Value` / `toml::Value`. [`From`]/[`TryFrom`]
//! fire only when a layer's type is not the output format — never as a hidden IR.

use std::fmt;

use knf_merge::{json_kind, toml_kind};
use serde_json::{Map, Value};

/// A JSON document. Newtype so [`TryFrom`]/`From` can sit on a local type.
#[derive(Debug, Clone, PartialEq)]
pub struct Json(pub Value);

/// A TOML document. Newtype so [`TryFrom`]/`From` can sit on a local type.
#[derive(Debug, Clone, PartialEq)]
pub struct Toml(pub toml::Value);

/// A parsed layer, still in its native format.
#[derive(Debug, Clone, PartialEq)]
pub enum Document {
    Json(Json),
    Toml(Toml),
}

impl Json {
    pub fn is_object(&self) -> bool {
        self.0.is_object()
    }

    pub fn kind(&self) -> &'static str {
        json_kind(&self.0)
    }
}

impl Toml {
    pub fn is_object(&self) -> bool {
        self.0.is_table()
    }

    pub fn kind(&self) -> &'static str {
        toml_kind(&self.0)
    }
}

impl From<Toml> for Json {
    fn from(Toml(value): Toml) -> Self {
        Json(toml_to_json(value))
    }
}

impl TryFrom<Json> for Toml {
    type Error = NullInToml;

    fn try_from(Json(value): Json) -> Result<Self, Self::Error> {
        let mut paths = Vec::new();
        collect_nulls(&value, &mut Vec::new(), &mut paths);
        if !paths.is_empty() {
            return Err(NullInToml {
                entries: paths.into_iter().map(|path| (path, None)).collect(),
            });
        }
        Ok(Toml(json_to_toml(&value)))
    }
}

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(i.into()),
        toml::Value::Float(f) => {
            Value::Number(serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()))
        }
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let mut map = Map::new();
            for (k, v) in table {
                map.insert(k, toml_to_json(v));
            }
            Value::Object(map)
        }
    }
}

fn json_to_toml(value: &Value) -> toml::Value {
    match value {
        Value::Null => {
            unreachable!("nulls are rejected by TryFrom before conversion");
        }
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => number_to_toml(n),
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().map(json_to_toml).collect()),
        Value::Object(map) => {
            let mut table = toml::Table::new();
            for (k, v) in map {
                table.insert(k.clone(), json_to_toml(v));
            }
            toml::Value::Table(table)
        }
    }
}

fn number_to_toml(n: &serde_json::Number) -> toml::Value {
    if let Some(i) = n.as_i64() {
        toml::Value::Integer(i)
    } else if let Some(u) = n.as_u64() {
        if u <= i64::MAX as u64 {
            toml::Value::Integer(u as i64)
        } else {
            toml::Value::Float(u as f64)
        }
    } else if let Some(f) = n.as_f64() {
        toml::Value::Float(f)
    } else {
        toml::Value::Float(0.0)
    }
}

// --- nulls in TOML --------------------------------------------------------

/// One step of a path into a JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Key(String),
    Index(usize),
}

fn render_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

fn collect_nulls(value: &Value, cur: &mut Vec<Seg>, out: &mut Vec<Vec<Seg>>) {
    match value {
        Value::Null => out.push(cur.clone()),
        Value::Object(obj) => {
            for (k, v) in obj {
                cur.push(Seg::Key(k.clone()));
                collect_nulls(v, cur, out);
                cur.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                cur.push(Seg::Index(i));
                collect_nulls(v, cur, out);
                cur.pop();
            }
        }
        _ => {}
    }
}

fn resolve<'a>(value: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    let mut cur = value;
    for seg in path {
        cur = match seg {
            Seg::Key(k) => cur.as_object()?.get(k)?,
            Seg::Index(i) => cur.as_array()?.get(*i)?,
        };
    }
    Some(cur)
}

/// Nulls survived into a document being converted to TOML.
///
/// A genuine impossibility in user data, so it is an error rather than a silent
/// drop. Provenance is filled in after the fact by looking each path up in the
/// JSON layers that went into the merge — not threaded through the merge.
#[derive(Debug)]
pub struct NullInToml {
    /// Path segments, and the last source that wrote null there (once attached).
    entries: Vec<(Vec<Seg>, Option<String>)>,
}

impl NullInToml {
    /// Look each null path up in `sources` and name the last layer that wrote it.
    pub fn with_origins<S: fmt::Display>(mut self, sources: &[(S, Json)]) -> Self {
        for (path, origin) in &mut self.entries {
            *origin = sources
                .iter()
                .rev()
                .find(|(_, v)| resolve(&v.0, path) == Some(&Value::Null))
                .map(|(name, _)| name.to_string());
        }
        self
    }
}

impl fmt::Display for NullInToml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "cannot serialize null to TOML")?;
        let rendered: Vec<(String, &Option<String>)> = self
            .entries
            .iter()
            .map(|(path, origin)| (render_path(path), origin))
            .collect();
        let width = rendered
            .iter()
            .map(|(path, _)| path.len())
            .max()
            .unwrap_or(0);
        for (path, origin) in &rendered {
            match origin {
                Some(origin) => writeln!(f, "  --> {path:<width$}   (from {origin})")?,
                None => writeln!(f, "  --> {path}")?,
            }
        }
        write!(f, "help: emit JSON with -f json, or remove the null")
    }
}

impl std::error::Error for NullInToml {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn datetime_becomes_a_json_string() {
        let parsed: toml::Value =
            toml::from_str("date = 1979-05-27T07:32:00Z\n").expect("valid toml");
        let Json(v) = Json::from(Toml(parsed));
        assert_eq!(v["date"], json!("1979-05-27T07:32:00Z"));
        assert!(v.get("$__toml_private_datetime").is_none());
    }

    #[test]
    fn json_to_toml_rejects_nulls_with_array_indices() {
        let err =
            Toml::try_from(Json(json!({"a": {"b": null}, "c": [1, null], "d": 2}))).unwrap_err();
        let rendered: Vec<_> = err.entries.iter().map(|(p, _)| render_path(p)).collect();
        assert_eq!(rendered, vec!["a.b", "c[1]"]);
    }

    #[test]
    fn numbers_bools_and_tables_round_trip() {
        let src = Json(json!({
            "n": 1,
            "f": 1.5,
            "ok": true,
            "name": "svc",
            "xs": [1, 2],
            "nested": {"k": 3}
        }));
        let toml = Toml::try_from(src.clone()).expect("no nulls");
        let Json(back) = Json::from(toml);
        assert_eq!(back, src.0);
    }

    #[test]
    fn with_origins_names_the_last_writer() {
        let err = Toml::try_from(Json(json!({"proxy": null}))).unwrap_err();
        let sources = [
            ("base.json", Json(json!({"proxy": null}))),
            ("over.json", Json(json!({"proxy": null}))),
        ];
        let err = err.with_origins(&sources);
        let msg = err.to_string();
        assert!(msg.contains("from over.json"), "{msg}");
        assert!(!msg.contains("from base.json"), "{msg}");
    }
}
