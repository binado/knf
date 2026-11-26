use knf::{Value, ValueKind};
use std::path::{Path, PathBuf};
use std::{fmt, io};

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

/// Why input paths could not be turned into configs.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// There are no inputs and no fallback format.
    #[error("no input files to infer a format from")]
    NoInputs,
    /// Inferred input formats differ.
    #[error("inputs mix JSON and TOML formats; configs must use one format")]
    MixedFormats,
    /// A positional named a directory.
    #[error("`{}` is a directory; knf takes files as configs", path.display())]
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
    /// A file could not be read or parsed.
    #[error("loading `{}`", path.display())]
    File {
        /// The file that failed.
        path: PathBuf,
        source: ReadError,
    },
}

/// Why one file could not be turned into a document.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The file could not be read, including when it is not UTF-8.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The contents are not a valid document.
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Why text is not a configuration document.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Invalid JSON syntax.
    #[error("invalid JSON")]
    Json(#[source] serde_json::Error),
    /// Invalid TOML syntax.
    #[error("invalid TOML")]
    Toml(#[source] Box<toml::de::Error>),
    /// The document root is not an object.
    #[error("expected an object at the top level, found {kind}")]
    NotObject {
        /// The root's kind, as named by [`Value::kind`].
        kind: ValueKind,
    },
}

/// Parsed documents in input order, plus the interpolation context.
#[derive(Debug, Clone, PartialEq)]
pub struct Configs {
    pub format: InputFormat,
    pub values: Vec<Value>,
    pub context: Option<Value>,
}

/// Resolve a single format from file extensions, without reading any contents.
///
/// Directory inputs are rejected before inference. No inputs resolve to
/// `fallback`, or fail with [`LoadError::NoInputs`].
pub fn resolve_format<P: AsRef<Path>>(
    paths: impl IntoIterator<Item = P>,
    fallback: Option<InputFormat>,
) -> Result<InputFormat, LoadError> {
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
    selected.or(fallback).ok_or(LoadError::NoInputs)
}

/// What [`load_configs`] reads besides the configs themselves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadOptions<'a> {
    /// Interpolation context file, read in the same format as the configs.
    pub context: Option<&'a Path>,
    /// The format when there are no inputs at all.
    pub fallback: Option<InputFormat>,
}

/// Infer one format for `files` and `options.context`, then read them in order.
pub fn load_configs<P: AsRef<Path>>(
    files: &[P],
    options: LoadOptions,
) -> Result<Configs, LoadError> {
    let LoadOptions { context, fallback } = options;
    let paths = files.iter().map(AsRef::as_ref).chain(context);
    let format = resolve_format(paths, fallback)?;
    let load = |path: &Path| {
        format.load(path).map_err(|source| LoadError::File {
            path: path.to_path_buf(),
            source,
        })
    };
    Ok(Configs {
        format,
        values: files
            .iter()
            .map(|path| load(path.as_ref()))
            .collect::<Result<_, _>>()?,
        context: context.map(load).transpose()?,
    })
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

    /// Read one document. The error does not repeat `path`.
    pub fn load(self, path: &Path) -> Result<Value, ReadError> {
        let text = std::fs::read_to_string(path)?;
        Ok(self.parse_document(&text)?)
    }

    /// Parse a configuration document with an object root.
    pub fn parse_document(self, text: &str) -> Result<Value, ParseError> {
        let value = match self {
            Self::Json => serde_json::from_str(text)
                .map(super::from_json)
                .map_err(ParseError::Json)?,
            Self::Toml => toml::from_str(text)
                .map(super::from_toml)
                .map_err(|source| ParseError::Toml(Box::new(source)))?,
        };
        if value.as_object().is_none() {
            return Err(ParseError::NotObject { kind: value.kind() });
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
