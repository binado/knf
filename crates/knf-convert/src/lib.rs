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

type Result<T> = std::result::Result<T, ConversionError>;

/// Converts TOML to JSON. When `strict` is false, dates, times and datetimes
/// become strings in TOML's standard notation, and non-finite floats become
/// `"nan"`, `"inf"` or `"-inf"`. Strict conversion rejects those mappings.
pub fn toml_to_json(value: Toml, strict: bool) -> Result<Json> {
    Converter {
        strict,
        path: Vec::new(),
    }
    .convert_to_json(value)
}

/// Converts JSON to TOML. When `strict` is false, null object fields are
/// omitted and integers above `i64::MAX` become floats. Strict conversion
/// rejects those mappings. Null array entries and root null always fail.
pub fn json_to_toml(value: Json, strict: bool) -> Result<Toml> {
    Converter {
        strict,
        path: Vec::new(),
    }
    .convert_to_toml(value)
}

struct Converter {
    strict: bool,
    path: Vec<Seg>,
}

impl Converter {
    fn error(&self, reason: &'static str) -> ConversionError {
        let path = if self.path.is_empty() {
            "<root>".into()
        } else {
            render_path(&self.path)
        };
        ConversionError { path, reason }
    }

    fn allow_loss(&self, reason: &'static str) -> Result<()> {
        if self.strict {
            return Err(self.error(reason));
        }
        Ok(())
    }

    fn at<T>(&mut self, segment: Seg, convert: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.path.push(segment);
        let result = convert(self);
        self.path.pop();
        result
    }

    fn convert_to_json(&mut self, value: Toml) -> Result<Json> {
        Ok(match value {
            Toml::Boolean(value) => Json::Bool(value),
            Toml::Integer(value) => Json::Number(value.into()),
            Toml::Float(value) => match Number::from_f64(value) {
                Some(number) => Json::Number(number),
                None => {
                    self.allow_loss("JSON cannot represent non-finite floats")?;
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
                self.allow_loss("JSON cannot represent TOML dates, times or datetimes")?;
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
                    if value.is_u64() {
                        self.allow_loss("integer exceeds TOML's signed 64-bit range")?;
                    }
                    Toml::Float(value.as_f64().expect("JSON numbers fit in f64 as floats"))
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
