use anyhow::{Context, bail};
use knf::Value;
use std::fmt;
use std::path::{Path, PathBuf};

/// Supported configuration formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Json,
    Toml,
}

impl fmt::Display for InputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// Why input paths could not be turned into layers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// Inferred input formats differ.
    #[error("inputs mix JSON and TOML formats; layers must use one format")]
    MixedFormats,
    /// A positional named a directory.
    #[error("`{}` is a directory; knf takes files as layers", path.display())]
    Directory {
        /// The directory that was named.
        path: PathBuf,
    },
    /// A positional's extension is neither `json` nor `toml`.
    #[error("cannot infer a format from `{}`", path.display())]
    UnknownExtension {
        /// The file whose extension said nothing.
        path: PathBuf,
    },
}

impl LoadError {
    /// The path this failed on, or `None` for mixed formats.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::MixedFormats => None,
            Self::Directory { path } | Self::UnknownExtension { path } => Some(path),
        }
    }
}

/// Parsed documents in input order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Layers {
    pub format: InputFormat,
    pub values: Vec<Value>,
}

impl Layers {
    /// Bundle documents already parsed as `format`.
    pub fn new(format: InputFormat, values: Vec<Value>) -> Self {
        Self { format, values }
    }
}

/// Resolve a single format from file extensions, without reading any contents.
///
/// Directory inputs are rejected before inference. No inputs default to JSON.
pub fn resolve_format<P: AsRef<Path>>(paths: &[P]) -> Result<InputFormat, LoadError> {
    let mut selected = None;
    for path in paths {
        let path = path.as_ref();
        if path.is_dir() {
            return Err(LoadError::Directory {
                path: path.to_path_buf(),
            });
        }
        let format = InputFormat::from_path(path).ok_or_else(|| LoadError::UnknownExtension {
            path: path.to_path_buf(),
        })?;
        if selected.is_some_and(|previous| previous != format) {
            return Err(LoadError::MixedFormats);
        }
        selected = Some(format);
    }
    Ok(selected.unwrap_or(InputFormat::Json))
}

/// Infer one format before reading the ordered documents.
pub fn load_layers<P: AsRef<Path>>(paths: &[P]) -> anyhow::Result<Layers> {
    let format = resolve_format(paths)?;
    let values = paths
        .iter()
        .map(|path| format.load(path.as_ref()))
        .collect::<anyhow::Result<_>>()?;
    Ok(Layers::new(format, values))
}

impl InputFormat {
    /// Infers a format from a file extension, case-insensitively.
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

    /// Read one document with filename-aware diagnostics.
    pub fn load(self, path: &Path) -> anyhow::Result<Value> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading `{}`", path.display()))?;
        self.parse_named(&text, &format!("{}: ", path.display()))
    }

    /// Parse a configuration document with an object root.
    pub fn parse_document(self, text: &str) -> anyhow::Result<Value> {
        self.parse_named(text, "")
    }

    fn parse_named(self, text: &str, prefix: &str) -> anyhow::Result<Value> {
        let value = match self {
            Self::Json => super::from_json(
                serde_json::from_str(text).with_context(|| format!("{prefix}invalid JSON"))?,
            ),
            Self::Toml => super::from_toml(
                toml::from_str(text).with_context(|| format!("{prefix}invalid TOML"))?,
            ),
        };
        if value.as_object().is_none() {
            bail!(
                "{prefix}expected an object at the top level, found {}",
                value.kind()
            )
        }
        Ok(value)
    }

    /// Parse one literal, preserving the original string when parsing fails.
    ///
    /// JSON parses the text as a JSON value. TOML trims surrounding ASCII
    /// whitespace and parses a TOML value, so `null` stays a string there.
    pub fn parse_inline(self, text: String) -> Value {
        match self {
            Self::Json => serde_json::from_str(&text)
                .map(super::from_json)
                .unwrap_or(Value::String(text)),
            Self::Toml => text
                .trim_matches([' ', '\t', '\r', '\n'])
                .parse()
                .map(super::from_toml)
                .unwrap_or(Value::String(text)),
        }
    }
}
