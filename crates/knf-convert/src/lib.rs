//! Convert native JSON and TOML values after configuration resolution.

use knf::{Seg, render_path};
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

/// Converts TOML to JSON. Under [`Policy::Lossy`], dates, times and datetimes
/// become strings in TOML's standard notation, and non-finite floats become
/// `"nan"`, `"inf"` or `"-inf"`. [`Policy::Strict`] rejects those mappings.
pub fn toml_to_json(value: Toml, policy: Policy) -> Result<Converted<Json>> {
    let mut converter = Converter::new(policy);
    let value = converter.convert_to_json(value)?;
    Ok(converter.finish(value))
}

/// Prepares TOML for YAML output. Under [`Policy::Lossy`], dates, times and
/// datetimes become strings in TOML's standard notation. Every other value,
/// including non-finite floats, is kept, and the result contains no
/// datetimes. [`Policy::Strict`] rejects the date mapping.
pub fn toml_to_yaml_ready(value: Toml, policy: Policy) -> Result<Converted<Toml>> {
    let mut converter = Converter::new(policy);
    let value = converter.stringify_datetimes(value)?;
    Ok(converter.finish(value))
}

/// Serializes a JSON value or a YAML-ready TOML value as YAML.
#[cfg(feature = "yaml")]
pub fn emit_yaml(value: &impl serde::Serialize) -> anyhow::Result<String> {
    use anyhow::Context;
    serde_saphyr::to_string(value).context("emitting YAML")
}

/// Converts JSON to TOML. Under [`Policy::Lossy`], null object fields are
/// omitted and integers above `i64::MAX` become floats. Strict conversion
/// rejects those mappings. Null array entries and root null always fail.
pub fn json_to_toml(value: Json, policy: Policy) -> Result<Converted<Toml>> {
    let mut converter = Converter::new(policy);
    let value = converter.convert_to_toml(value)?;
    Ok(converter.finish(value))
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

    fn stringify_datetimes(&mut self, value: Toml) -> Result<Toml> {
        Ok(match value {
            Toml::Datetime(value) => {
                self.allow_loss("YAML output writes TOML dates, times and datetimes as strings")?;
                Toml::String(value.to_string())
            }
            Toml::Array(values) => Toml::Array(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.at(Seg::Index(index), |this| this.stringify_datetimes(value))
                    })
                    .collect::<Result<_>>()?,
            ),
            Toml::Table(values) => Toml::Table(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let value = self.at(Seg::Key(key.clone()), |this| {
                            this.stringify_datetimes(value)
                        })?;
                        Ok((key, value))
                    })
                    .collect::<Result<_>>()?,
            ),
            other => other,
        })
    }

    fn convert_to_json(&mut self, value: Toml) -> Result<Json> {
        Ok(match value {
            Toml::Boolean(value) => Json::Bool(value),
            Toml::Integer(value) => Json::Number(value.into()),
            Toml::Float(value) => match Number::from_f64(value) {
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
            Toml::String(value) => Json::String(value),
            Toml::Datetime(value) => {
                self.allow_loss(
                    "the JSON data model cannot represent TOML dates, times or datetimes",
                )?;
                Json::String(value.to_string())
            }
            Toml::Array(values) => Json::Array(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.at(Seg::Index(index), |this| this.convert_to_json(value))
                    })
                    .collect::<Result<_>>()?,
            ),
            Toml::Table(values) => Json::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let value =
                            self.at(Seg::Key(key.clone()), |this| this.convert_to_json(value))?;
                        Ok((key, value))
                    })
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn convert_to_toml(&mut self, value: Json) -> Result<Toml> {
        Ok(match value {
            Json::Null => return Err(self.error("TOML cannot represent null")),
            Json::Bool(value) => Toml::Boolean(value),
            Json::Number(value) => match value.as_i64() {
                Some(integer) => Toml::Integer(integer),
                None => {
                    if !value.is_f64() {
                        self.allow_loss("integer exceeds TOML's signed 64-bit range")?;
                    }
                    match value.as_f64() {
                        Some(float) => Toml::Float(float),
                        None => return Err(self.error("number cannot be represented in TOML")),
                    }
                }
            },
            Json::String(value) => Toml::String(value),
            Json::Array(values) => Toml::Array(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.at(Seg::Index(index), |this| this.convert_to_toml(value))
                    })
                    .collect::<Result<_>>()?,
            ),
            Json::Object(values) => {
                let mut table = toml::Table::new();
                for (key, value) in values {
                    if value.is_null() {
                        self.at(Seg::Key(key), |this| {
                            this.allow_loss("TOML cannot represent null object fields")
                        })?;
                    } else {
                        let value =
                            self.at(Seg::Key(key.clone()), |this| this.convert_to_toml(value))?;
                        table.insert(key, value);
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
    fn yaml_ready_toml_keeps_floats_and_stringifies_datetimes() {
        let converted = toml_to_yaml_ready(
            toml("a = nan\nb = [inf, 1979-05-27]\n[t]\nc = 07:32:00"),
            Policy::Lossy,
        )
        .unwrap();
        assert!(converted.lossy);
        let table = converted.value.as_table().unwrap();
        assert!(table["a"].as_float().unwrap().is_nan());
        assert_eq!(table["b"][0].as_float(), Some(f64::INFINITY));
        assert_eq!(table["b"][1].as_str(), Some("1979-05-27"));
        assert_eq!(table["t"]["c"].as_str(), Some("07:32:00"));
    }

    #[test]
    fn yaml_ready_toml_without_datetimes_is_lossless_and_strict_names_the_key() {
        let converted = toml_to_yaml_ready(toml("a = nan"), Policy::Strict).unwrap();
        assert!(!converted.lossy);
        assert_eq!(
            err(toml_to_yaml_ready(
                toml("[n]\nd = 1979-05-27"),
                Policy::Strict
            )),
            "n.d: YAML output writes TOML dates, times and datetimes as strings"
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
