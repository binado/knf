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
        return Err(NullInToml { entries: paths });
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

/// Substitutes `placeholder` for every null in the document.
///
/// The alternative to [`to_toml`]'s rejection, and so only ever called on the
/// way to TOML — JSON holds a null fine and has nothing to be rescued from. A
/// null cannot be encoded as TOML, leaving only two honest options: fail, or
/// write a value that was in none of the inputs. *Which* value that is has to
/// be the user's choice rather than the tool's — `yq` and `tomlq` both drop
/// null keys silently, and both invent a string inside arrays (`""` and
/// `"None"` respectively, for the same input), which is the behaviour this
/// exists to avoid.
pub fn replace_nulls(value: &mut Value, placeholder: &str) {
    match value {
        Value::Null => *value = Value::String(placeholder.to_string()),
        Value::Object(obj) => {
            for v in obj.values_mut() {
                replace_nulls(v, placeholder);
            }
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                replace_nulls(v, placeholder);
            }
        }
        _ => {}
    }
}

/// Nulls survived into a document being converted to TOML.
///
/// A genuine impossibility in user data, so it is an error rather than a silent
/// drop: the `toml` crate's map serializer *skips* a `None` entry, so emitting
/// without this check would quietly lose keys.
///
/// Carries paths and nothing else. Naming the layer each null came from would
/// mean retaining every parsed layer past the merge purely for an error path;
/// the paths alone locate the value in the merged document, and the usual fix
/// (`-f json`, or `--null-as`) does not depend on knowing the file.
#[derive(Debug)]
pub struct NullInToml {
    entries: Vec<Vec<Seg>>,
}

impl fmt::Display for NullInToml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "cannot serialize null to TOML")?;
        for path in &self.entries {
            writeln!(f, "  --> {}", render_path(path))?;
        }
        write!(
            f,
            "help: emit JSON with -f json, substitute with --null-as, or remove the null"
        )
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
        let rendered: Vec<_> = err.entries.iter().map(|p| render_path(p)).collect();
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

    /// The array case is the one both `yq` and `tomlq` fabricate a value for,
    /// since a null cannot be dropped from an array without shifting every
    /// index after it. Substituting preserves the length and lets the user name
    /// the value that lands there.
    #[test]
    fn replace_nulls_substitutes_everywhere_and_unblocks_toml() {
        let mut v = ir(json!({"a": {"b": null}, "c": [1, null, 3], "d": 2}));
        replace_nulls(&mut v, "none");
        assert_eq!(
            to_json(v.clone()),
            json!({"a": {"b": "none"}, "c": [1, "none", 3], "d": 2})
        );
        to_toml(v).expect("the substitution left no nulls");
    }

    /// A document without nulls is untouched, so the flag cannot perturb a
    /// merge that never needed it.
    #[test]
    fn replace_nulls_is_the_identity_without_nulls() {
        let src = json!({"a": 1, "xs": [1, 2], "nested": {"k": "v"}});
        let mut v = ir(src.clone());
        replace_nulls(&mut v, "none");
        assert_eq!(to_json(v), src);
    }
}
