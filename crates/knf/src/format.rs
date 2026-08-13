//! Format detection, parsing and emission.
//!
//! Everything format-specific lives here. The merge core sees only
//! `serde_json::Value` and knows nothing about TOML's datetimes or its
//! inability to represent null.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::Value;

/// v1 ships JSON and TOML only. Adding a format is one arm of these matches;
/// removing one is a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Json,
    Toml,
}

impl Format {
    /// Infers a format from a file extension. `None` means "no opinion" — the
    /// caller decides whether that is an error or a cue to fall back.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("json") {
            Some(Self::Json)
        } else if ext.eq_ignore_ascii_case("toml") {
            Some(Self::Toml)
        } else {
            None
        }
    }

    /// The extension `matrix` gives files it writes.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// Where a layer came from. Used only for error messages — provenance is never
/// threaded through the merge itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceName {
    File(PathBuf),
    Stdin,
    Set(String),
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(p) => write!(f, "{}", p.display()),
            Self::Stdin => f.write_str("<stdin>"),
            Self::Set(expr) => write!(f, "--set {expr}"),
        }
    }
}

/// A parsed layer, retained after merging so that emit-time errors can point at
/// the file that wrote the offending value (§3.2).
pub type Source = (SourceName, Value);

/// Parses one layer.
///
/// Enforces §2.3: every input must be an object at the top level. A bare array
/// or string root is legal JSON but is not a config, cannot be emitted as TOML,
/// and produces nonsense under last-wins.
pub fn parse(format: Format, text: &str, source: &SourceName) -> anyhow::Result<Value> {
    let value: Value = match format {
        Format::Json => {
            serde_json::from_str(text).with_context(|| format!("{source}: invalid JSON"))?
        }
        Format::Toml => toml::from_str(text).with_context(|| format!("{source}: invalid TOML"))?,
    };
    if !value.is_object() {
        bail!(
            "{source}: expected an object at the top level, found {}",
            knf_merge::kind(&value)
        );
    }
    Ok(value)
}

/// Serializes the merged document.
///
/// `sources` are the parsed inputs, retained so a null-in-TOML failure can name
/// the file responsible. Takes the value by move because the JSON path rewrites
/// datetime sentinels in place.
pub fn emit(
    format: Format,
    value: Value,
    pretty: bool,
    sources: &[Source],
) -> anyhow::Result<String> {
    let text = match format {
        Format::Json => {
            let mut value = value;
            strip_datetimes(&mut value);
            if pretty {
                serde_json::to_string_pretty(&value)?
            } else {
                serde_json::to_string(&value)?
            }
        }
        Format::Toml => {
            // Pre-check rather than interpreting serde's error: toml's map
            // serializer *skips* entries that serialize as None, so relying on
            // the serializer would silently drop nulls instead of failing.
            if let Some(err) = NullInToml::detect(&value, sources) {
                return Err(err.into());
            }
            let doc = TomlValue(&value);
            if pretty {
                toml::to_string_pretty(&doc)?
            } else {
                toml::to_string(&doc)?
            }
        }
    };
    Ok(ensure_trailing_newline(text))
}

fn ensure_trailing_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

// --- datetimes ------------------------------------------------------------

/// `toml`'s deserializer turns a datetime into a one-key map under this
/// sentinel, and its serializer recognises a datetime by *struct name* rather
/// than by this key. The constant is `pub(crate)` in `toml_datetime`, so there
/// is nothing to import.
const DATETIME_KEY: &str = "$__toml_private_datetime";

/// Recognises the sentinel map `toml` produces for a datetime.
///
/// Deliberately strict: exactly one key, that key, and a value that actually
/// parses as a datetime. A hand-written object that merely happens to use the
/// key is left alone, so the check never changes the meaning of user data.
fn as_datetime(value: &Value) -> Option<toml::value::Datetime> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get(DATETIME_KEY)?.as_str()?.parse().ok()
}

/// Replaces datetime sentinels with their plain string form, for JSON output.
///
/// Without this a TOML input silently produces `{"$__toml_private_datetime":
/// "1979-05-27T07:32:00Z"}` in the JSON — an internal detail leaking into user
/// output.
fn strip_datetimes(value: &mut Value) {
    match value {
        Value::Object(_) => {
            if let Some(dt) = as_datetime(value) {
                *value = Value::String(dt.to_string());
                return;
            }
            let Value::Object(obj) = value else {
                unreachable!("just matched an object");
            };
            for (_, v) in obj.iter_mut() {
                strip_datetimes(v);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_datetimes),
        _ => {}
    }
}

/// Serializes a `Value` as TOML, converting datetime sentinels back into real
/// datetimes on the way out.
///
/// The conversion cannot happen anywhere else. `toml`'s serializer detects a
/// datetime by the struct name `Datetime::serialize` passes to
/// `serialize_struct`, and a `serde_json::Map` always goes through
/// `serialize_map` — so re-inserting the sentinel key would emit a sub-table
/// with a bizarre key, not a datetime. Delegating to `Datetime::serialize`
/// here is what makes TOML -> TOML lossless.
struct TomlValue<'a>(&'a Value);

impl Serialize for TomlValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            // Unreachable in practice: `emit` pre-checks for nulls. Kept
            // faithful rather than clever in case a caller skips the check.
            Value::Null => serializer.serialize_none(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Number(n) => n.serialize(serializer),
            Value::String(s) => serializer.serialize_str(s),
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&TomlValue(item))?;
                }
                seq.end()
            }
            Value::Object(obj) => {
                if let Some(dt) = as_datetime(self.0) {
                    return dt.serialize(serializer);
                }
                let mut map = serializer.serialize_map(Some(obj.len()))?;
                for (k, v) in obj {
                    map.serialize_entry(k, &TomlValue(v))?;
                }
                map.end()
            }
        }
    }
}

// --- nulls in TOML --------------------------------------------------------

/// One step of a path into a `Value`.
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

/// Nulls survived into a document being emitted as TOML.
///
/// A genuine impossibility in user data, so it is an error rather than a silent
/// drop. serde's own message ("unsupported None value") has no key path, which
/// would leave the user bisecting a merged tree by hand — hence the post-hoc
/// lookup that turns each path back into the file that wrote it.
#[derive(Debug)]
pub struct NullInToml {
    /// Rendered key path, and the last source that wrote null there.
    entries: Vec<(String, Option<String>)>,
}

impl NullInToml {
    fn detect(value: &Value, sources: &[Source]) -> Option<Self> {
        let mut paths = Vec::new();
        collect_nulls(value, &mut Vec::new(), &mut paths);
        if paths.is_empty() {
            return None;
        }
        let entries = paths
            .into_iter()
            .map(|path| {
                // Reverse: the *last* layer that wrote null there is the one
                // that put it in the merged document.
                let origin = sources
                    .iter()
                    .rev()
                    .find(|(_, v)| resolve(v, &path) == Some(&Value::Null))
                    .map(|(name, _)| name.to_string());
                (render_path(&path), origin)
            })
            .collect();
        Some(Self { entries })
    }
}

impl fmt::Display for NullInToml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "cannot serialize null to TOML")?;
        let width = self
            .entries
            .iter()
            .map(|(path, _)| path.len())
            .max()
            .unwrap_or(0);
        for (path, origin) in &self.entries {
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
    fn extension_inference() {
        assert_eq!(Format::from_path(Path::new("a.json")), Some(Format::Json));
        assert_eq!(Format::from_path(Path::new("a.TOML")), Some(Format::Toml));
        assert_eq!(Format::from_path(Path::new("a.yaml")), None);
        assert_eq!(Format::from_path(Path::new("a")), None);
    }

    #[test]
    fn top_level_must_be_an_object() {
        let err = parse(Format::Json, "[1,2]", &SourceName::Stdin).unwrap_err();
        assert!(err.to_string().contains("found array"), "{err}");
    }

    #[test]
    fn datetime_sentinel_is_recognised_precisely() {
        assert!(as_datetime(&json!({ DATETIME_KEY: "1979-05-27T07:32:00Z" })).is_some());
        // Not a datetime: extra key, wrong key, unparseable value.
        assert!(as_datetime(&json!({ DATETIME_KEY: "1979-05-27T07:32:00Z", "x": 1 })).is_none());
        assert!(as_datetime(&json!({ "when": "1979-05-27T07:32:00Z" })).is_none());
        assert!(as_datetime(&json!({ DATETIME_KEY: "not a datetime" })).is_none());
    }

    #[test]
    fn null_paths_include_array_indices() {
        let mut out = Vec::new();
        collect_nulls(
            &json!({"a": {"b": null}, "c": [1, null], "d": 2}),
            &mut Vec::new(),
            &mut out,
        );
        let rendered: Vec<_> = out.iter().map(|p| render_path(p)).collect();
        assert_eq!(rendered, vec!["a.b", "c[1]"]);
    }
}
