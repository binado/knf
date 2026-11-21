//! Load ordered configuration layers and emit resolved [`Value`]s.
//!
//! # Example
//!
//! With `base.toml` and `prod.toml` in the working directory:
//! ```no_run
#![doc = include_str!("../examples/toml_to_json.rs")]
//! ```

mod input;
pub use input::{InputFormat, Layers, LoadError, load_layers, resolve_format};
use knf::{Seg, Value, render_path};
use serde::ser::{self, Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value as Json;
use std::cell::Cell;
use toml::Value as Toml;

/// A value that cannot be converted under the requested policy.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {reason}")]
pub struct ConversionError {
    path: String,
    kind: &'static str,
    reason: &'static str,
}

impl ConversionError {
    /// Key path of the offending value, or `<root>`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Kind of the offending value, as named by [`Value::kind`].
    pub fn kind(&self) -> &str {
        self.kind
    }

    /// Why the value cannot be converted.
    pub fn reason(&self) -> &str {
        self.reason
    }
}

/// Whether lossy mappings are applied or rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Apply lossy mappings and report them in [`Converted::lossy`].
    Lossy,
    /// Reject lossy mappings with a [`ConversionError`].
    Strict,
}

impl Policy {
    /// [`Policy::Strict`] when `strict`, else [`Policy::Lossy`].
    pub fn strict(strict: bool) -> Self {
        if strict { Self::Strict } else { Self::Lossy }
    }
}

/// A converted value and whether any mapping was lossy.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted<T> {
    pub value: T,
    /// True when a lossy mapping was applied; always false under [`Policy::Strict`].
    pub lossy: bool,
}

/// Whitespace in emitted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Single-line JSON and compact TOML.
    Compact,
    /// Indented, human-readable output.
    Pretty,
}

impl Layout {
    /// [`Layout::Pretty`] when `pretty`, else [`Layout::Compact`].
    pub fn pretty(pretty: bool) -> Self {
        if pretty { Self::Pretty } else { Self::Compact }
    }
}

/// Output encoding and conversion rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputFormat {
    Json,
    Toml,
    #[cfg(feature = "yaml")]
    Yaml,
}
impl From<InputFormat> for OutputFormat {
    fn from(format: InputFormat) -> Self {
        match format {
            InputFormat::Json => Self::Json,
            InputFormat::Toml => Self::Toml,
        }
    }
}

impl OutputFormat {
    /// Encode a value with a trailing newline, reporting any lossy mappings.
    ///
    /// With [`Policy::Lossy`], JSON and YAML stringify datetimes. JSON also
    /// stringifies non-finite floats. TOML omits null fields and converts
    /// oversized unsigned integers to floats. A null array entry or root
    /// cannot be emitted as TOML under either policy.
    pub fn emit(
        self,
        value: &Value,
        policy: Policy,
        layout: Layout,
    ) -> anyhow::Result<Converted<String>> {
        use anyhow::Context;
        let pretty = layout == Layout::Pretty;
        let emitter = Emitter {
            format: self,
            policy,
            lossy: Cell::new(false),
            error: Cell::new(None),
        };
        let root = Node {
            emitter: &emitter,
            value,
            parent: None,
        };
        let written = match self {
            Self::Json => if pretty {
                serde_json::to_string_pretty(&root)
            } else {
                serde_json::to_string(&root)
            }
            .context("emitting JSON"),
            Self::Toml => if pretty {
                toml::to_string_pretty(&root)
            } else {
                toml::to_string(&root)
            }
            .context("emitting TOML"),
            #[cfg(feature = "yaml")]
            Self::Yaml => serde_saphyr::to_string(&root).context("emitting YAML"),
        };
        if let Some(error) = emitter.error.take() {
            return Err(error.into());
        }
        let mut text = written?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(Converted {
            value: text,
            lossy: emitter.lossy.get(),
        })
    }

    /// How this format writes `value`; containers are always [`Repr::Native`].
    fn represent(self, value: &Value) -> Repr<'_> {
        match (self, value) {
            (Self::Json, Value::Float(f)) if !f.is_finite() => Repr::Lossy {
                reason: "the JSON data model cannot represent non-finite floats",
                fallback: Fallback::Str(if f.is_nan() {
                    "nan"
                } else if f.is_sign_negative() {
                    "-inf"
                } else {
                    "inf"
                }),
            },
            (Self::Json, Value::Datetime(dt)) => Repr::Lossy {
                reason: "the JSON data model cannot represent TOML dates, times or datetimes",
                fallback: Fallback::Datetime(dt),
            },
            #[cfg(feature = "yaml")]
            (Self::Yaml, Value::Datetime(dt)) => Repr::Lossy {
                reason: "YAML output writes TOML dates, times and datetimes as strings",
                fallback: Fallback::Datetime(dt),
            },
            (Self::Toml, Value::Null) => Repr::Rejected("TOML cannot represent null"),
            (Self::Toml, Value::Unsigned(u)) => Repr::Lossy {
                reason: "integer exceeds TOML's signed 64-bit range",
                fallback: Fallback::Float(u.get() as f64),
            },
            _ => Repr::Native,
        }
    }

    /// Why this format drops an object field under [`Policy::Lossy`], or
    /// `None` to keep it.
    fn omits_field(self, value: &Value) -> Option<&'static str> {
        (self == Self::Toml && matches!(value, Value::Null))
            .then_some("TOML cannot represent null object fields")
    }
}

/// Convert a JSON document; integers that fit `i64` become `Integer`, larger ones `Unsigned`.
/// Never yields `Datetime`.
pub fn from_json(value: Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(b),
        Json::String(s) => Value::String(s),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else if let Some(u) = n.as_u64() {
                Value::unsigned(u)
            } else {
                n.as_f64()
                    .map_or_else(|| Value::String(n.to_string()), Value::Float)
            }
        }
        Json::Array(a) => Value::Array(a.into_iter().map(from_json).collect()),
        Json::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

/// Convert a TOML document, keeping datetimes and non-finite floats. Never yields `Null`.
pub fn from_toml(value: Toml) -> Value {
    match value {
        Toml::Boolean(b) => Value::Bool(b),
        Toml::Integer(i) => Value::Integer(i),
        Toml::Float(f) => Value::Float(f),
        Toml::String(s) => Value::String(s),
        Toml::Datetime(d) => Value::Datetime(d),
        Toml::Array(a) => Value::Array(a.into_iter().map(from_toml).collect()),
        Toml::Table(o) => Value::Object(o.into_iter().map(|(k, v)| (k, from_toml(v))).collect()),
    }
}

/// How a format writes one value.
enum Repr<'a> {
    /// Written unchanged.
    Native,
    /// Written as `fallback` under [`Policy::Lossy`], rejected under [`Policy::Strict`].
    Lossy {
        reason: &'static str,
        fallback: Fallback<'a>,
    },
    /// Rejected under either policy.
    Rejected(&'static str),
}

/// The replacement written for a lossy value.
enum Fallback<'a> {
    Str(&'static str),
    Float(f64),
    Datetime(&'a toml::value::Datetime),
}

impl Serialize for Fallback<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Str(text) => serializer.serialize_str(text),
            Self::Float(f) => serializer.serialize_f64(*f),
            Self::Datetime(dt) => serializer.serialize_str(&dt.to_string()),
        }
    }
}

/// State shared by one [`OutputFormat::emit`] call.
struct Emitter {
    format: OutputFormat,
    policy: Policy,
    lossy: Cell<bool>,
    /// The conversion failure; serializer errors only carry its message.
    error: Cell<Option<ConversionError>>,
}

/// A value under serialization, linked to its parent so its path is only
/// built on error.
struct Node<'n, 'a> {
    emitter: &'n Emitter,
    value: &'a Value,
    parent: Option<(&'n Node<'n, 'a>, Step<'a>)>,
}

#[derive(Clone, Copy)]
enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

impl<'n, 'a> Node<'n, 'a> {
    fn child(&'n self, step: Step<'a>, value: &'a Value) -> Self {
        Node {
            emitter: self.emitter,
            value,
            parent: Some((self, step)),
        }
    }

    fn path(&self) -> String {
        let mut segments = Vec::new();
        let mut node = self;
        while let Some((parent, step)) = node.parent {
            segments.push(match step {
                Step::Key(key) => Seg::Key(key.to_owned()),
                Step::Index(index) => Seg::Index(index),
            });
            node = parent;
        }
        if segments.is_empty() {
            return "<root>".into();
        }
        segments.reverse();
        render_path(&segments)
    }

    /// Record the conversion failure and abort serialization.
    fn fail<E: ser::Error>(&self, reason: &'static str) -> E {
        let error = ConversionError {
            path: self.path(),
            kind: self.value.kind(),
            reason,
        };
        let abort = E::custom(&error);
        self.emitter.error.set(Some(error));
        abort
    }

    fn allow_loss<E: ser::Error>(&self, reason: &'static str) -> Result<(), E> {
        match self.emitter.policy {
            Policy::Strict => Err(self.fail(reason)),
            Policy::Lossy => {
                self.emitter.lossy.set(true);
                Ok(())
            }
        }
    }
}

impl Serialize for Node<'_, '_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let format = self.emitter.format;
        match format.represent(self.value) {
            Repr::Native => {}
            Repr::Lossy { reason, fallback } => {
                self.allow_loss(reason)?;
                return fallback.serialize(serializer);
            }
            Repr::Rejected(reason) => return Err(self.fail(reason)),
        }
        match self.value {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Integer(i) => serializer.serialize_i64(*i),
            Value::Unsigned(u) => serializer.serialize_u64(u.get()),
            Value::Float(f) => serializer.serialize_f64(*f),
            Value::String(s) => serializer.serialize_str(s),
            Value::Datetime(dt) => dt.serialize(serializer),
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for (index, item) in items.iter().enumerate() {
                    seq.serialize_element(&self.child(Step::Index(index), item))?;
                }
                seq.end()
            }
            Value::Object(fields) => {
                let kept = fields
                    .values()
                    .filter(|value| format.omits_field(value).is_none())
                    .count();
                let mut map = serializer.serialize_map(Some(kept))?;
                for (key, value) in fields {
                    let child = self.child(Step::Key(key), value);
                    match format.omits_field(value) {
                        Some(reason) => child.allow_loss(reason)?,
                        None => map.serialize_entry(key, &child)?,
                    }
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn toml(text: &str) -> Toml {
        toml::from_str::<toml::Table>(text).unwrap().into()
    }

    fn emit(
        format: OutputFormat,
        value: Value,
        policy: Policy,
    ) -> anyhow::Result<Converted<String>> {
        format.emit(&value, policy, Layout::Compact)
    }

    fn json_out(out: &Converted<String>) -> Json {
        serde_json::from_str(&out.value).unwrap()
    }

    fn toml_out(out: &Converted<String>) -> Toml {
        toml(&out.value)
    }

    fn err(result: anyhow::Result<Converted<String>>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn toml_lossy_values_become_strings() {
        let input = toml(
            "[nested]\ndate=1979-05-27\ntime=07:32:00\nlocal=1979-05-27T07:32:00\n\
             offset=1979-05-27T07:32:00-07:00\nfloats=[nan, inf, -inf, -nan]\n",
        );
        let out = emit(OutputFormat::Json, from_toml(input), Policy::Lossy).unwrap();
        assert!(out.lossy);
        assert_eq!(
            json_out(&out),
            json!({"nested": {
                "date": "1979-05-27",
                "time": "07:32:00",
                "local": "1979-05-27T07:32:00",
                "offset": "1979-05-27T07:32:00-07:00",
                "floats": ["nan", "inf", "-inf", "nan"]
            }})
        );
    }

    #[test]
    fn toml_lossless_values_are_not_flagged() {
        let input = from_toml(toml("a = 1\nb = [1.5, 'x', true]\n"));
        let out = emit(OutputFormat::Json, input, Policy::Strict).unwrap();
        assert!(!out.lossy);
        assert_eq!(json_out(&out), json!({"a": 1, "b": [1.5, "x", true]}));
    }

    #[test]
    fn strict_toml_rejects_each_lossy_kind_with_its_path() {
        for (text, path) in [
            ("[n]\nv=1979-05-27", "n.v"),
            ("[n]\nv=07:32:00", "n.v"),
            ("[n]\nv=1979-05-27T07:32:00", "n.v"),
            ("[n]\nv=1979-05-27T07:32:00-07:00", "n.v"),
            ("[n]\nv=[1, nan]", "n.v[1]"),
            ("[n]\nv=[inf]", "n.v[0]"),
            ("[n]\nv=[-inf]", "n.v[0]"),
            ("a=[{b=[1979-05-27]}]", "a[0].b[0]"),
        ] {
            let message = err(emit(
                OutputFormat::Json,
                from_toml(toml(text)),
                Policy::Strict,
            ));
            assert!(
                message.starts_with(&format!("{path}: ")),
                "{text}: {message}"
            );
        }
    }

    #[test]
    fn json_lossy_values_are_adjusted() {
        let input = json!({"nested": {"drop": null, "large": u64::MAX, "float": 1.5}});
        let out = emit(OutputFormat::Toml, from_json(input), Policy::Lossy).unwrap();
        assert!(out.lossy);
        let out = toml_out(&out);
        let nested = out["nested"].as_table().unwrap();
        assert!(!nested.contains_key("drop"));
        assert_eq!(nested["large"].as_float(), Some(u64::MAX as f64));
        assert_eq!(nested["float"].as_float(), Some(1.5));
    }

    #[test]
    fn json_all_null_object_becomes_empty_table() {
        let input = from_json(json!({"a": {"b": null}}));
        let out = emit(OutputFormat::Toml, input, Policy::Lossy).unwrap();
        assert!(out.lossy);
        assert_eq!(toml_out(&out), toml("[a]\n"));
    }

    #[test]
    fn json_integer_boundaries() {
        for bound in [i64::MIN, i64::MAX] {
            let input = from_json(json!({ "v": bound }));
            let out = emit(OutputFormat::Toml, input, Policy::Strict).unwrap();
            assert_eq!(toml_out(&out)["v"].as_integer(), Some(bound));
        }
        let over = || from_json(json!({"v": i64::MAX as u64 + 1}));
        assert!(
            emit(OutputFormat::Toml, over(), Policy::Lossy)
                .unwrap()
                .lossy
        );
        assert_eq!(
            err(emit(OutputFormat::Toml, over(), Policy::Strict)),
            "v: integer exceeds TOML's signed 64-bit range"
        );
    }

    #[test]
    fn json_lossless_values_are_not_flagged() {
        let input = from_json(json!({"a": [1, {"b": 2.5}], "c": "x", "d": false}));
        let out = emit(OutputFormat::Toml, input, Policy::Strict).unwrap();
        assert!(!out.lossy);
    }

    #[test]
    fn strict_json_rejects_each_lossy_kind_with_its_path() {
        for (input, message) in [
            (
                json!({"n": {"drop": null}}),
                "n.drop: TOML cannot represent null object fields",
            ),
            (
                json!({"a": [{"b": u64::MAX}]}),
                "a[0].b: integer exceeds TOML's signed 64-bit range",
            ),
        ] {
            let result = emit(OutputFormat::Toml, from_json(input), Policy::Strict);
            assert_eq!(err(result), message);
        }
    }

    #[test]
    fn json_null_in_arrays_and_at_root_fails_in_every_mode() {
        for policy in [Policy::Lossy, Policy::Strict] {
            assert_eq!(
                err(emit(
                    OutputFormat::Toml,
                    from_json(json!({"a": [1, null]})),
                    policy
                )),
                "a[1]: TOML cannot represent null"
            );
            assert_eq!(
                err(emit(OutputFormat::Toml, Value::Null, policy)),
                "<root>: TOML cannot represent null"
            );
        }
    }

    #[test]
    fn error_exposes_path_and_reason() {
        let input = from_json(json!({"a": [null]}));
        let error = emit(OutputFormat::Toml, input, Policy::Lossy).unwrap_err();
        let error = error.downcast_ref::<ConversionError>().unwrap();
        assert_eq!(error.path(), "a[0]");
        assert_eq!(error.kind(), "null");
        assert_eq!(error.reason(), "TOML cannot represent null");
    }

    #[test]
    fn error_paths_skip_successful_siblings() {
        let input = from_json(json!({"ok": {"x": [1]}, "bad": [null]}));
        assert_eq!(
            err(emit(OutputFormat::Toml, input, Policy::Lossy)),
            "bad[0]: TOML cannot represent null"
        );
    }
}
