//! Format detection, parsing and emission.
//!
//! Both directions cross the IR boundary here and nowhere else: parse yields a
//! [`Value`], emit takes one. The conversions themselves live in
//! [`crate::value`].

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use knf_core::Value;

use crate::value;

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

    /// The canonical file extension for this format.
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

/// Which input a parse error came from. Names an input being *read*, so there
/// is no variant for `--set`: a bad `--set` expression is rejected by
/// `knf-dotted` during argument parsing, long before anything reaches here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceName {
    File(PathBuf),
    Stdin,
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(p) => write!(f, "{}", p.display()),
            Self::Stdin => f.write_str("<stdin>"),
        }
    }
}

/// Parses one layer into the merge IR.
///
/// Enforces §2.3: every input must be an object at the top level. A bare array
/// or string root is legal JSON but is not a config, cannot be emitted as TOML,
/// and produces nonsense under last-wins.
pub fn parse(format: Format, text: &str, source: &SourceName) -> anyhow::Result<Value> {
    let value = match format {
        Format::Json => {
            let native: serde_json::Value =
                serde_json::from_str(text).with_context(|| format!("{source}: invalid JSON"))?;
            value::from_json(native)
        }
        Format::Toml => {
            let native: toml::Value =
                toml::from_str(text).with_context(|| format!("{source}: invalid TOML"))?;
            value::from_toml(native)
        }
    };
    if !matches!(value, Value::Object(_)) {
        bail!(
            "{source}: expected an object at the top level, found {}",
            value.kind()
        );
    }
    Ok(value)
}

/// Converts the merged IR into `format` and serializes it.
///
/// `null_as` substitutes a string for every null rather than failing on one.
/// It is honoured in the TOML arm and nowhere else: JSON can hold a null
/// perfectly well, so there is nothing there for it to rescue and substituting
/// anyway would corrupt a document that was never in trouble.
///
/// The null pre-check inside [`value::to_toml`] is then the only thing that can
/// fail here, and it reports key paths alone — nothing about the inputs
/// survives the merge for it to name.
pub fn emit(
    value: Value,
    format: Format,
    pretty: bool,
    null_as: Option<&str>,
) -> anyhow::Result<String> {
    let text = match format {
        Format::Json => {
            let native = value::to_json(value);
            if pretty {
                serde_json::to_string_pretty(&native)?
            } else {
                serde_json::to_string(&native)?
            }
        }
        Format::Toml => {
            let mut value = value;
            if let Some(placeholder) = null_as {
                value::replace_nulls(&mut value, placeholder);
            }
            let native = value::to_toml(value)?;
            if pretty {
                toml::to_string_pretty(&native)?
            } else {
                toml::to_string(&native)?
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
