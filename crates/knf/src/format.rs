//! Format detection, parsing and emission.
//!
//! Parse yields a native [`Document`]. Emit serializes that document as-is.
//! JSON↔TOML type conversion lives in [`crate::value`], not here.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use knf_merge::MergeValue;

use crate::value::{Document, Json, Toml};

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

/// Parses one layer into its native document type.
///
/// Enforces §2.3: every input must be an object at the top level. A bare array
/// or string root is legal JSON but is not a config, cannot be emitted as TOML,
/// and produces nonsense under last-wins.
pub fn parse(format: Format, text: &str, source: &SourceName) -> anyhow::Result<Document> {
    match format {
        Format::Json => {
            let value: serde_json::Value =
                serde_json::from_str(text).with_context(|| format!("{source}: invalid JSON"))?;
            let json = Json(value);
            if !json.is_object() {
                bail!(
                    "{source}: expected an object at the top level, found {}",
                    json.kind()
                );
            }
            Ok(Document::Json(json))
        }
        Format::Toml => {
            let value: toml::Value =
                toml::from_str(text).with_context(|| format!("{source}: invalid TOML"))?;
            let toml = Toml(value);
            if !toml.is_object() {
                bail!(
                    "{source}: expected an object at the top level, found {}",
                    toml.kind()
                );
            }
            Ok(Document::Toml(toml))
        }
    }
}

/// Serializes a native document. No JSON↔TOML conversion happens here.
pub fn emit(doc: Document, pretty: bool) -> anyhow::Result<String> {
    let text = match doc {
        Document::Json(Json(value)) => {
            if pretty {
                serde_json::to_string_pretty(&value)?
            } else {
                serde_json::to_string(&value)?
            }
        }
        Document::Toml(Toml(value)) => {
            if pretty {
                toml::to_string_pretty(&value)?
            } else {
                toml::to_string(&value)?
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
