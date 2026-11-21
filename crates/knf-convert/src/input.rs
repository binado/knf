use knf::Value;
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

/// Why input paths could not be turned into layers.
#[derive(Debug)]
pub enum LoadError {
    /// There are no inputs and no fallback format.
    NoInputs,
    /// Inferred input formats differ.
    MixedFormats,
    /// A positional named a directory.
    Directory {
        /// The directory that was named.
        path: PathBuf,
    },
    /// A positional's extension is neither `json` nor `toml`.
    UnknownExtension {
        /// The file whose extension said nothing.
        path: PathBuf,
    },
    /// A file could not be read or parsed. Displays as `source`.
    File {
        /// The file that failed.
        path: PathBuf,
        /// The read or parse failure, already naming `path`.
        source: ReadError,
    },
}

/// Why one file could not be turned into a document.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The file could not be read, including when it is not UTF-8.
    #[error("reading `{}`", path.display())]
    Io {
        /// The file that failed.
        path: PathBuf,
        source: io::Error,
    },
    /// The contents are not a valid document.
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Why text is not a configuration document. Displays prefixed with the
/// file name when there is one.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// Invalid JSON syntax.
    #[error("{}invalid JSON", Prefix(path))]
    Json {
        /// The file that was parsed, if any.
        path: Option<PathBuf>,
        source: serde_json::Error,
    },
    /// Invalid TOML syntax.
    #[error("{}invalid TOML", Prefix(path))]
    Toml {
        /// The file that was parsed, if any.
        path: Option<PathBuf>,
        source: Box<toml::de::Error>,
    },
    /// The document root is not an object.
    #[error("{}expected an object at the top level, found {kind}", Prefix(path))]
    NotObject {
        /// The file that was parsed, if any.
        path: Option<PathBuf>,
        /// The root's kind, as named by [`Value::kind`].
        kind: &'static str,
    },
}

/// Renders `path: `, or nothing without a path.
struct Prefix<'a>(&'a Option<PathBuf>);

impl fmt::Display for Prefix<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(path) => write!(f, "{}: ", path.display()),
            None => Ok(()),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoInputs => f.write_str("no input files to infer a format from"),
            Self::MixedFormats => {
                f.write_str("inputs mix JSON and TOML formats; layers must use one format")
            }
            Self::Directory { path } => write!(
                f,
                "`{}` is a directory; knf takes files as layers",
                path.display()
            ),
            Self::UnknownExtension { path } => {
                write!(f, "cannot infer a format from `{}`", path.display())
            }
            Self::File { source, .. } => fmt::Display::fmt(source, f),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::File { source, .. } => source.source(),
            Self::NoInputs
            | Self::MixedFormats
            | Self::Directory { .. }
            | Self::UnknownExtension { .. } => None,
        }
    }
}

/// Parsed documents in input order, plus the interpolation context.
#[derive(Debug, Clone, PartialEq)]
pub struct Layers {
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

/// Infer one format for `files` and `context`, then read them in order.
///
/// `fallback` is the format when there are no inputs at all.
pub fn load_layers<P: AsRef<Path>>(
    files: &[P],
    context: Option<&Path>,
    fallback: Option<InputFormat>,
) -> Result<Layers, LoadError> {
    let paths = files.iter().map(AsRef::as_ref).chain(context);
    let format = resolve_format(paths, fallback)?;
    let load = |path: &Path| {
        format.load(path).map_err(|source| LoadError::File {
            path: path.to_path_buf(),
            source,
        })
    };
    Ok(Layers {
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

    /// Read one document with filename-aware diagnostics.
    pub fn load(self, path: &Path) -> Result<Value, ReadError> {
        let text = std::fs::read_to_string(path).map_err(|source| ReadError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(self.parse_named(&text, Some(path))?)
    }

    /// Parse a configuration document with an object root.
    pub fn parse_document(self, text: &str) -> Result<Value, ParseError> {
        self.parse_named(text, None)
    }

    fn parse_named(self, text: &str, path: Option<&Path>) -> Result<Value, ParseError> {
        let path = || path.map(Path::to_path_buf);
        let value = match self {
            Self::Json => serde_json::from_str(text)
                .map(super::from_json)
                .map_err(|source| ParseError::Json {
                    path: path(),
                    source,
                })?,
            Self::Toml => toml::from_str(text)
                .map(super::from_toml)
                .map_err(|source| ParseError::Toml {
                    path: path(),
                    source: Box::new(source),
                })?,
        };
        if value.as_object().is_none() {
            return Err(ParseError::NotObject {
                path: path(),
                kind: value.kind(),
            });
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
