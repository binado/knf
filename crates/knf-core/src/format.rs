//! Format detection, parsing and emission.
//!
//! Values remain native throughout parsing, merging and emission.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::ConfigValue;
use anyhow::{Context, bail};

/// The two supported native formats. A pipeline selects one for every stage.
///
/// No `clap::ValueEnum` here — this crate has no clap. `knf-cli` parses `-f`
/// into a local enum and converts, which the orphan rule would force anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// [`PathLeaf`](crate::PathLeaf) during argument parsing, long before anything
/// reaches here.
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

mod private {
    pub trait Sealed {}
    impl Sealed for serde_json::Value {}
    impl Sealed for toml::Value {}
}

/// Parsing and emission for the two supported native formats.
///
/// Sealed to JSON and TOML; structural algorithms use the open
/// [`ConfigValue`] interface. Inline values and whole-string environment
/// references always share this trait's typing rule.
pub trait ConfigFormat: ConfigValue + private::Sealed {
    /// The format represented by this native value.
    const FORMAT: Format;
    /// Parse a document. Top-level object validation happens in [`parse`].
    fn parse_document(text: &str) -> anyhow::Result<Self>;
    /// Parse one value, falling back to the original text as a string.
    fn parse_inline(text: String) -> Self;
    /// Serialize directly in this native format.
    fn serialize(&self, pretty: bool) -> anyhow::Result<String>;
}

impl ConfigFormat for serde_json::Value {
    const FORMAT: Format = Format::Json;
    fn parse_document(text: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
    fn parse_inline(text: String) -> Self {
        crate::set::json_or_string(text)
    }
    fn serialize(&self, pretty: bool) -> anyhow::Result<String> {
        Ok(if pretty {
            serde_json::to_string_pretty(self)?
        } else {
            serde_json::to_string(self)?
        })
    }
}

impl ConfigFormat for toml::Value {
    const FORMAT: Format = Format::Toml;
    fn parse_document(text: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(text)?)
    }
    fn parse_inline(text: String) -> Self {
        crate::set::toml_or_string(text)
    }
    fn serialize(&self, pretty: bool) -> anyhow::Result<String> {
        Ok(if pretty {
            toml::to_string_pretty(self)?
        } else {
            toml::to_string(self)?
        })
    }
}

/// Parse a native document, requiring an object/table root.
pub fn parse<V: ConfigFormat>(text: &str, source: &SourceName) -> anyhow::Result<V> {
    let label = match V::FORMAT {
        Format::Json => "JSON",
        Format::Toml => "TOML",
    };
    let value = V::parse_document(text).with_context(|| format!("{source}: invalid {label}"))?;
    if value.as_object().is_none() {
        bail!(
            "{source}: expected an object at the top level, found {}",
            value.kind()
        );
    }
    Ok(value)
}

/// Emit a native value in its own format, with a trailing newline.
pub fn emit<V: ConfigFormat>(value: V, pretty: bool) -> anyhow::Result<String> {
    Ok(ensure_trailing_newline(value.serialize(pretty)?))
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
        let err = parse::<serde_json::Value>("[1,2]", &SourceName::Stdin).unwrap_err();
        assert!(err.to_string().contains("found array"), "{err}");
    }
}
