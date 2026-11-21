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
use serde_json::{Number, Value as Json};
use toml::Value as Toml;

/// A value that cannot be converted under the requested policy.
#[derive(Debug, thiserror::Error)]
#[error("{path}: {reason}")]
pub struct ConversionError {
    path: String,
    reason: &'static str,
}

impl ConversionError {
    /// Key path of the offending value, or `<root>`.
    pub fn path(&self) -> &str {
        &self.path
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

type Result<T> = std::result::Result<T, ConversionError>;

/// A converted value and whether any mapping was lossy.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted<T> {
    pub value: T,
    /// True when a lossy mapping was applied; always false under [`Policy::Strict`].
    pub lossy: bool,
}

/// Output encoding and conversion rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        pretty: bool,
    ) -> anyhow::Result<Converted<String>> {
        let mut converter = Converter::new(policy);
        let mut text = match self {
            Self::Json => {
                let native = converter.convert_to_json(value)?;
                if pretty {
                    serde_json::to_string_pretty(&native)?
                } else {
                    serde_json::to_string(&native)?
                }
            }
            Self::Toml => {
                let native = converter.convert_to_toml(value)?;
                if pretty {
                    toml::to_string_pretty(&native)?
                } else {
                    toml::to_string(&native)?
                }
            }
            #[cfg(feature = "yaml")]
            Self::Yaml => {
                use anyhow::Context;
                serde_saphyr::to_string(&converter.convert_to_yaml(value)?)
                    .context("emitting YAML")?
            }
        };
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(converter.finish(text))
    }
}

/// Preserve the native JSON value and normalize integers that fit in i64.
pub fn from_json(value: Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(b),
        Json::String(s) => Value::String(s),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else if let Some(u) = n.as_u64() {
                Value::Unsigned(u)
            } else {
                Value::Float(n.as_f64().expect("JSON number"))
            }
        }
        Json::Array(a) => Value::Array(a.into_iter().map(from_json).collect()),
        Json::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

/// Preserve the native TOML value, including datetimes and non-finite floats.
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

#[cfg(feature = "yaml")]
#[derive(serde::Serialize)]
#[serde(untagged)]
enum Yaml<'a> {
    Null,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    String(std::borrow::Cow<'a, str>),
    Array(Vec<Yaml<'a>>),
    Object(indexmap::IndexMap<&'a str, Yaml<'a>>),
}

struct Converter {
    policy: Policy,
    lossy: bool,
    path: Vec<Seg>,
}

impl Converter {
    fn new(policy: Policy) -> Self {
        Self {
            policy,
            lossy: false,
            path: Vec::new(),
        }
    }

    fn finish<T>(self, value: T) -> Converted<T> {
        Converted {
            value,
            lossy: self.lossy,
        }
    }

    fn error(&self, reason: &'static str) -> ConversionError {
        let path = if self.path.is_empty() {
            "<root>".into()
        } else {
            render_path(&self.path)
        };
        ConversionError { path, reason }
    }

    fn allow_loss(&mut self, reason: &'static str) -> Result<()> {
        if self.policy == Policy::Strict {
            return Err(self.error(reason));
        }
        self.lossy = true;
        Ok(())
    }

    fn at<T>(&mut self, segment: Seg, convert: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.path.push(segment);
        let result = convert(self);
        self.path.pop();
        result
    }

    #[cfg(feature = "yaml")]
    fn convert_to_yaml<'a>(&mut self, value: &'a Value) -> Result<Yaml<'a>> {
        Ok(match value {
            Value::Null => Yaml::Null,
            Value::Bool(b) => Yaml::Bool(*b),
            Value::Integer(i) => Yaml::Integer(*i),
            Value::Unsigned(u) => Yaml::Unsigned(*u),
            Value::Float(f) => Yaml::Float(*f),
            Value::String(s) => Yaml::String(s.as_str().into()),
            Value::Datetime(dt) => {
                self.allow_loss("YAML output writes TOML dates, times and datetimes as strings")?;
                Yaml::String(dt.to_string().into())
            }
            Value::Array(items) => Yaml::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| self.at(Seg::Index(i), |this| this.convert_to_yaml(v)))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(map) => Yaml::Object(
                map.iter()
                    .map(|(k, v)| {
                        Ok((
                            k.as_str(),
                            self.at(Seg::Key(k.clone()), |this| this.convert_to_yaml(v))?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn convert_to_json(&mut self, value: &Value) -> Result<Json> {
        Ok(match value {
            Value::Null => Json::Null,
            Value::Unsigned(value) => Json::Number((*value).into()),
            Value::Bool(value) => Json::Bool(*value),
            Value::Integer(value) => Json::Number((*value).into()),
            Value::Float(value) => match Number::from_f64(*value) {
                Some(number) => Json::Number(number),
                None => {
                    self.allow_loss("the JSON data model cannot represent non-finite floats")?;
                    let text = if value.is_nan() {
                        "nan"
                    } else if value.is_sign_negative() {
                        "-inf"
                    } else {
                        "inf"
                    };
                    Json::String(text.into())
                }
            },
            Value::String(value) => Json::String(value.clone()),
            Value::Datetime(value) => {
                self.allow_loss(
                    "the JSON data model cannot represent TOML dates, times or datetimes",
                )?;
                Json::String(value.to_string())
            }
            Value::Array(values) => Json::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.at(Seg::Index(index), |this| this.convert_to_json(value))
                    })
                    .collect::<Result<_>>()?,
            ),
            Value::Object(values) => Json::Object(
                values
                    .iter()
                    .map(|(key, value)| {
                        let value =
                            self.at(Seg::Key(key.clone()), |this| this.convert_to_json(value))?;
                        Ok((key.clone(), value))
                    })
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn convert_to_toml(&mut self, value: &Value) -> Result<Toml> {
        Ok(match value {
            Value::Null => return Err(self.error("TOML cannot represent null")),
            Value::Bool(value) => Toml::Boolean(*value),
            Value::Integer(value) => Toml::Integer(*value),
            Value::Unsigned(value) => match i64::try_from(*value) {
                Ok(i) => Toml::Integer(i),
                Err(_) => {
                    self.allow_loss("integer exceeds TOML's signed 64-bit range")?;
                    Toml::Float(*value as f64)
                }
            },
            Value::Float(value) => Toml::Float(*value),
            Value::Datetime(value) => Toml::Datetime(*value),
            Value::String(value) => Toml::String(value.clone()),
            Value::Array(values) => Toml::Array(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.at(Seg::Index(index), |this| this.convert_to_toml(value))
                    })
                    .collect::<Result<_>>()?,
            ),
            Value::Object(values) => {
                let mut table = toml::Table::new();
                for (key, value) in values {
                    if matches!(value, Value::Null) {
                        self.at(Seg::Key(key.clone()), |this| {
                            this.allow_loss("TOML cannot represent null object fields")
                        })?;
                    } else {
                        let value =
                            self.at(Seg::Key(key.clone()), |this| this.convert_to_toml(value))?;
                        table.insert(key.clone(), value);
                    }
                }
                Toml::Table(table)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn toml(text: &str) -> Toml {
        toml::from_str::<toml::Table>(text).unwrap().into()
    }

    fn json_to_toml(value: Json, policy: Policy) -> Result<Converted<Toml>> {
        let mut c = Converter::new(policy);
        let native = c.convert_to_toml(&from_json(value))?;
        Ok(c.finish(native))
    }
    fn toml_to_json(value: Toml, policy: Policy) -> Result<Converted<Json>> {
        let mut c = Converter::new(policy);
        let native = c.convert_to_json(&from_toml(value))?;
        Ok(c.finish(native))
    }
    fn err(result: Result<impl std::fmt::Debug>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn toml_lossy_values_become_strings() {
        let input = toml(
            "[nested]\ndate=1979-05-27\ntime=07:32:00\nlocal=1979-05-27T07:32:00\n\
             offset=1979-05-27T07:32:00-07:00\nfloats=[nan, inf, -inf, -nan]\n",
        );
        let out = toml_to_json(input, Policy::Lossy).unwrap();
        assert!(out.lossy);
        assert_eq!(
            out.value,
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
        let out = toml_to_json(toml("a = 1\nb = [1.5, 'x', true]\n"), Policy::Strict).unwrap();
        assert!(!out.lossy);
        assert_eq!(out.value, json!({"a": 1, "b": [1.5, "x", true]}));
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
            let message = err(toml_to_json(toml(text), Policy::Strict));
            assert!(
                message.starts_with(&format!("{path}: ")),
                "{text}: {message}"
            );
        }
    }

    #[test]
    fn json_lossy_values_are_adjusted() {
        let input = json!({"nested": {"drop": null, "large": u64::MAX, "float": 1.5}});
        let out = json_to_toml(input, Policy::Lossy).unwrap();
        assert!(out.lossy);
        let nested = out.value["nested"].as_table().unwrap();
        assert!(!nested.contains_key("drop"));
        assert_eq!(nested["large"].as_float(), Some(u64::MAX as f64));
        assert_eq!(nested["float"].as_float(), Some(1.5));
    }

    #[test]
    fn json_all_null_object_becomes_empty_table() {
        let out = json_to_toml(json!({"a": {"b": null}}), Policy::Lossy).unwrap();
        assert!(out.lossy);
        assert_eq!(out.value, toml("[a]\n"));
    }

    #[test]
    fn json_integer_boundaries() {
        let min = json_to_toml(json!({"v": i64::MIN}), Policy::Strict).unwrap();
        assert_eq!(min.value["v"].as_integer(), Some(i64::MIN));
        let max = json_to_toml(json!({"v": i64::MAX}), Policy::Strict).unwrap();
        assert_eq!(max.value["v"].as_integer(), Some(i64::MAX));
        let over = json!({"v": i64::MAX as u64 + 1});
        assert!(json_to_toml(over.clone(), Policy::Lossy).unwrap().lossy);
        assert_eq!(
            err(json_to_toml(over, Policy::Strict)),
            "v: integer exceeds TOML's signed 64-bit range"
        );
    }

    #[test]
    fn json_lossless_values_are_not_flagged() {
        let input = json!({"a": [1, {"b": 2.5}], "c": "x", "d": false});
        let out = json_to_toml(input, Policy::Strict).unwrap();
        assert!(!out.lossy);
    }

    #[test]
    fn strict_json_rejects_each_lossy_kind_with_its_path() {
        assert_eq!(
            err(json_to_toml(json!({"n": {"drop": null}}), Policy::Strict)),
            "n.drop: TOML cannot represent null object fields"
        );
        assert_eq!(
            err(json_to_toml(
                json!({"a": [{"b": u64::MAX}]}),
                Policy::Strict
            )),
            "a[0].b: integer exceeds TOML's signed 64-bit range"
        );
    }

    #[test]
    fn json_null_in_arrays_and_at_root_fails_in_every_mode() {
        for policy in [Policy::Lossy, Policy::Strict] {
            assert_eq!(
                err(json_to_toml(json!({"a": [1, null]}), policy)),
                "a[1]: TOML cannot represent null"
            );
            assert_eq!(
                err(json_to_toml(Json::Null, policy)),
                "<root>: TOML cannot represent null"
            );
        }
    }

    #[test]
    fn error_exposes_path_and_reason() {
        let error = json_to_toml(json!({"a": [null]}), Policy::Lossy).unwrap_err();
        assert_eq!(error.path(), "a[0]");
        assert_eq!(error.reason(), "TOML cannot represent null");
    }

    #[test]
    fn error_paths_are_balanced_after_a_sibling_succeeds() {
        // A path leaked by a successful sibling would show up in the error.
        let message = err(json_to_toml(
            json!({"ok": {"x": [1]}, "bad": [null]}),
            Policy::Lossy,
        ));
        assert_eq!(message, "bad[0]: TOML cannot represent null");
    }
}
