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

/// What a path was supplied as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A config to merge.
    Input,
    /// The interpolation context.
    Context,
}

/// Why input paths could not be turned into configs.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// There are no inputs and no fallback format.
    #[error("no input files to infer a format from")]
    NoInputs,
    /// Inferred input formats differ.
    #[error(
        "`{}` is {} but `{}` is {}; configs must use one format",
        conflict.display(),
        conflict_format.name(),
        first.display(),
        first_format.name()
    )]
    MixedFormats {
        /// The first path, which set the expected format.
        first: PathBuf,
        first_format: InputFormat,
        /// The first path whose format differs.
        conflict: PathBuf,
        conflict_format: InputFormat,
        /// The role of `conflict`. Inputs are checked before the context.
        role: Role,
    },
    /// A path named a directory.
    #[error("`{}` is a directory; knf takes files as configs", path.display())]
    Directory {
        /// The directory that was named.
        path: PathBuf,
        role: Role,
    },
    /// A path's extension is neither `json` nor `toml`.
    #[error("cannot infer a format from `{}`", path.display())]
    UnknownExtension {
        /// The file whose extension said nothing.
        path: PathBuf,
        role: Role,
    },
    /// A file could not be read or parsed.
    #[error("loading `{}`", path.display())]
    File {
        /// The file that failed.
        path: PathBuf,
        role: Role,
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
/// `files` are checked in order, then `context`; the first bad path fails.
/// Directories are rejected before inference. With no paths at all, the format
/// is `fallback`, or the call fails with [`LoadError::NoInputs`].
pub fn resolve_format<P: AsRef<Path>>(
    files: impl IntoIterator<Item = P>,
    context: Option<&Path>,
    fallback: Option<InputFormat>,
) -> Result<InputFormat, LoadError> {
    let mut first: Option<(InputFormat, PathBuf)> = None;
    let mut check = |path: &Path, role| {
        if path.is_dir() {
            return Err(LoadError::Directory {
                path: path.to_path_buf(),
                role,
            });
        }
        let format = InputFormat::from_path(path).ok_or_else(|| LoadError::UnknownExtension {
            path: path.to_path_buf(),
            role,
        })?;
        match &first {
            None => first = Some((format, path.to_path_buf())),
            Some((first_format, first_path)) if *first_format != format => {
                return Err(LoadError::MixedFormats {
                    first: first_path.clone(),
                    first_format: *first_format,
                    conflict: path.to_path_buf(),
                    conflict_format: format,
                    role,
                });
            }
            Some(_) => {}
        }
        Ok(())
    };
    for file in files {
        check(file.as_ref(), Role::Input)?;
    }
    if let Some(path) = context {
        check(path, Role::Context)?;
    }
    first
        .map(|(format, _)| format)
        .or(fallback)
        .ok_or(LoadError::NoInputs)
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
    let format = resolve_format(files.iter().map(AsRef::as_ref), context, fallback)?;
    let load = |path: &Path, role| {
        format.load(path).map_err(|source| LoadError::File {
            path: path.to_path_buf(),
            role,
            source,
        })
    };
    Ok(Configs {
        format,
        values: files
            .iter()
            .map(|path| load(path.as_ref(), Role::Input))
            .collect::<Result<_, _>>()?,
        context: context.map(|path| load(path, Role::Context)).transpose()?,
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

    fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Toml => "TOML",
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
