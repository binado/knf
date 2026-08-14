//! Conversion between the merge IR and the native JSON/TOML value types.
//!
//! Merge runs on [`knf_core::Value`], so these fire exactly twice per run: once
//! per layer on the way in, once on the whole document on the way out. Free
//! functions rather than `From`/`TryFrom` impls because both sides are foreign
//! types — `impl From<toml::Value> for knf_core::Value` names nothing local and
//! does not compile.

use std::fmt;

use knf_core::{Number, Value};

/// JSON → IR. Total: every JSON value has an IR counterpart.
pub fn from_json(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(number_from_json(&n)),
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(items) => Value::Array(items.into_iter().map(from_json).collect()),
        serde_json::Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, from_json(v))).collect())
        }
    }
}

/// IR → JSON. Total; a datetime renders as the string JSON would have to use.
pub fn to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Number(n) => serde_json::Value::Number(number_to_json(n)),
        Value::String(s) | Value::Datetime(s) => serde_json::Value::String(s),
        Value::Array(items) => serde_json::Value::Array(items.into_iter().map(to_json).collect()),
        Value::Object(map) => {
            serde_json::Value::Object(map.into_iter().map(|(k, v)| (k, to_json(v))).collect())
        }
    }
}

/// TOML → IR. Total: datetimes keep their source spelling rather than becoming
/// strings, which is what lets a TOML datetime survive a mixed-format merge.
pub fn from_toml(value: toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(i) => Value::Number(Number::I64(i)),
        toml::Value::Float(f) => Value::Number(Number::F64(f)),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(dt) => Value::Datetime(dt.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(from_toml).collect()),
        toml::Value::Table(table) => {
            Value::Object(table.into_iter().map(|(k, v)| (k, from_toml(v))).collect())
        }
    }
}

/// IR → TOML, rejecting nulls first.
///
/// The pre-check is separate because serde's own message ("unsupported None
/// value") carries no key path, and `toml`'s map serializer *skips* `None`
/// entries rather than failing — so walking for nulls up front is the only way
/// to surface the impossibility at all, let alone with paths.
pub fn to_toml(value: Value) -> Result<toml::Value, NullInToml> {
    let mut paths = Vec::new();
    collect_nulls(&value, &mut Vec::new(), &mut paths);
    if !paths.is_empty() {
        return Err(NullInToml {
            entries: paths.into_iter().map(|path| (path, None)).collect(),
        });
    }
    Ok(to_toml_unchecked(value))
}

fn to_toml_unchecked(value: Value) -> toml::Value {
    match value {
        Value::Null => {
            unreachable!("nulls are rejected by to_toml before conversion");
        }
        Value::Bool(b) => toml::Value::Boolean(b),
        Value::Number(n) => number_to_toml(n),
        Value::String(s) => toml::Value::String(s),
        // Infallible by construction: the only producers of IR values are the
        // TOML parser, the JSON parser and `--set` (a JSON-parsed RHS), and only
        // the first ever emits `Datetime` — from a string `toml` itself printed.
        Value::Datetime(s) => toml::Value::Datetime(
            s.parse()
                .expect("a Datetime is only ever produced by the TOML parser, so it re-parses"),
        ),
        Value::Array(items) => {
            toml::Value::Array(items.into_iter().map(to_toml_unchecked).collect())
        }
        Value::Object(map) => {
            let mut table = toml::Table::new();
            for (k, v) in map {
                table.insert(k, to_toml_unchecked(v));
            }
            toml::Value::Table(table)
        }
    }
}

fn number_from_json(n: &serde_json::Number) -> Number {
    if let Some(i) = n.as_i64() {
        Number::I64(i)
    } else if let Some(u) = n.as_u64() {
        Number::from_u64(u)
    } else if let Some(f) = n.as_f64() {
        Number::F64(f)
    } else {
        Number::F64(0.0)
    }
}

fn number_to_json(n: Number) -> serde_json::Number {
    match n {
        Number::I64(i) => i.into(),
        Number::U64(u) => u.into(),
        Number::F64(f) => serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()),
    }
}

fn number_to_toml(n: Number) -> toml::Value {
    match n {
        Number::I64(i) => toml::Value::Integer(i),
        // TOML integers are signed, so anything past i64::MAX has to become a
        // float. Lossy, but the alternative is refusing to emit at all.
        Number::U64(u) => match i64::try_from(u) {
            Ok(i) => toml::Value::Integer(i),
            Err(_) => toml::Value::Float(u as f64),
        },
        Number::F64(f) => toml::Value::Float(f),
    }
}

// --- nulls in TOML --------------------------------------------------------

/// One step of a path into a value.
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
        cur = match (seg, cur) {
            (Seg::Key(k), Value::Object(obj)) => obj.get(k)?,
            (Seg::Index(i), Value::Array(items)) => items.get(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Nulls survived into a document being converted to TOML.
///
/// A genuine impossibility in user data, so it is an error rather than a silent
/// drop. Provenance is filled in after the fact by looking each path up in the
/// layers that went into the merge — not threaded through the merge.
#[derive(Debug)]
pub struct NullInToml {
    /// Path segments, and the last source that wrote null there (once attached).
    entries: Vec<(Vec<Seg>, Option<String>)>,
}

impl NullInToml {
    /// Look each null path up in `sources` and name the last layer that wrote it.
    pub fn with_origins<S: fmt::Display>(mut self, sources: &[(S, Value)]) -> Self {
        for (path, origin) in &mut self.entries {
            *origin = sources
                .iter()
                .rev()
                .find(|(_, v)| resolve(v, path) == Some(&Value::Null))
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

    fn ir(v: serde_json::Value) -> Value {
        from_json(v)
    }

    #[test]
    fn a_toml_datetime_stays_a_datetime_in_the_ir() {
        let parsed: toml::Value =
            toml::from_str("date = 1979-05-27T07:32:00Z\n").expect("valid toml");
        let v = from_toml(parsed);
        let Value::Object(map) = &v else {
            panic!("expected an object, got {v:?}");
        };
        assert_eq!(
            map["date"],
            Value::Datetime("1979-05-27T07:32:00Z".to_string())
        );
        // Only the sentinel-free `Display` spelling, never toml's internal map.
        assert_eq!(to_json(v), json!({"date": "1979-05-27T07:32:00Z"}));
    }

    /// All four TOML datetime forms round-trip through `Display`/`FromStr`,
    /// which is the whole basis for storing a datetime as a `String`.
    #[test]
    fn every_toml_datetime_form_round_trips() {
        let src = "\
offset = 1979-05-27T07:32:00Z
offset_frac = 1979-05-27T00:32:00.999999-07:00
local = 1979-05-27T07:32:00
date = 1979-05-27
time = 07:32:00.5
";
        let parsed: toml::Value = toml::from_str(src).expect("valid toml");
        let back = to_toml(from_toml(parsed.clone())).expect("no nulls");
        assert_eq!(back, parsed);
    }

    #[test]
    fn to_toml_rejects_nulls_with_array_indices() {
        let err = to_toml(ir(json!({"a": {"b": null}, "c": [1, null], "d": 2}))).unwrap_err();
        let rendered: Vec<_> = err.entries.iter().map(|(p, _)| render_path(p)).collect();
        assert_eq!(rendered, vec!["a.b", "c[1]"]);
    }

    #[test]
    fn numbers_bools_and_tables_round_trip() {
        let src = json!({
            "n": 1,
            "f": 1.5,
            "ok": true,
            "name": "svc",
            "xs": [1, 2],
            "nested": {"k": 3}
        });
        let toml = to_toml(ir(src.clone())).expect("no nulls");
        assert_eq!(to_json(from_toml(toml)), src);
    }

    /// A JSON integer above `i64::MAX` must not be rounded through `f64`.
    #[test]
    fn large_unsigned_integers_survive_json_round_trip() {
        let src = json!({"id": 10_000_000_000_000_000_001_u64});
        assert_eq!(to_json(ir(src.clone())), src);
    }

    #[test]
    fn with_origins_names_the_last_writer() {
        let err = to_toml(ir(json!({"proxy": null}))).unwrap_err();
        let sources = [
            ("base.json", ir(json!({"proxy": null}))),
            ("over.json", ir(json!({"proxy": null}))),
        ];
        let err = err.with_origins(&sources);
        let msg = err.to_string();
        assert!(msg.contains("from over.json"), "{msg}");
        assert!(!msg.contains("from base.json"), "{msg}");
    }
}
