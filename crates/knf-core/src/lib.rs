//! Load, merge and interpolate homogeneous JSON or TOML configuration layers.
//!
//! [`load_layers`] returns native [`Layers`]. Match its variant, then call
//! [`merge`], optionally [`interpolate`], and [`format::emit`] on that same
//! value type. No stage converts JSON to TOML or TOML to JSON.

pub mod format;
pub mod fs;
pub mod glob;
mod inline;
mod interp;
mod merge;
mod path;
pub mod value;

mod env;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub use env::ProcessEnv;
pub use format::{ConfigFormat, Format};
pub use inline::{PathLeaf, json_or_string, toml_or_string};
pub use interp::{Cycle, Env, InterpError, Problem, Syntax, interpolate};
pub use merge::{MergeError, MergeOptions, merge, merge_into};
pub use path::{PathError, RefPath, Seg, render_path};
pub use value::{ConfigObject, ConfigValue};

use format::SourceName;

/// The positional that means "read stdin".
pub const STDIN: &str = "-";

/// Why a positional could not be turned into a layer.
///
/// Reports source selection failures without frontend flag vocabulary.
/// Format mismatches have no single source path; other input failures do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// `-` was given without an explicit input format.
    #[error("`-` reads stdin, which has no extension")]
    StdinNeedsFormat,
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
    /// The path this failed on, or `None` for stdin and mixed formats.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::StdinNeedsFormat | Self::MixedFormats => None,
            Self::Directory { path } | Self::UnknownExtension { path } => Some(path),
        }
    }
}

/// Native layers, all in one format and in the caller's original order.
#[derive(Debug, Clone, PartialEq)]
pub enum Layers {
    /// JSON documents.
    Json(Vec<serde_json::Value>),
    /// TOML documents.
    Toml(Vec<toml::Value>),
}

/// Resolve a single format without reading any document contents.
///
/// An explicit format overrides extensions. Stdin requires an explicit format;
/// no inputs default to JSON. Directory inputs are rejected before inference.
pub fn resolve_format<P: AsRef<Path>>(
    paths: &[P],
    explicit: Option<Format>,
) -> Result<Format, LoadError> {
    let mut selected = explicit;
    for path in paths {
        let path = path.as_ref();
        if path.is_dir() {
            return Err(LoadError::Directory {
                path: path.to_path_buf(),
            });
        }
        if explicit.is_some() {
            continue;
        }
        let format = if path.as_os_str() == STDIN {
            return Err(LoadError::StdinNeedsFormat);
        } else {
            Format::from_path(path).ok_or_else(|| LoadError::UnknownExtension {
                path: path.to_path_buf(),
            })?
        };
        if selected.is_some_and(|previous| previous != format) {
            return Err(LoadError::MixedFormats);
        }
        selected = Some(format);
    }
    Ok(selected.unwrap_or(Format::Json))
}

/// Read homogeneous native layers after resolving one format for the entire list.
///
/// Stdin requires an explicit format. Mixed inferred formats fail before any
/// document contents are read. Empty input returns JSON layers unless overridden.
pub fn load_layers<P: AsRef<Path>>(
    paths: &[P],
    explicit_format: Option<Format>,
) -> anyhow::Result<Layers> {
    match resolve_format(paths, explicit_format)? {
        Format::Json => Ok(Layers::Json(load_native(paths)?)),
        Format::Toml => Ok(Layers::Toml(load_native(paths)?)),
    }
}

fn load_native<P: AsRef<Path>, V: ConfigFormat>(paths: &[P]) -> anyhow::Result<Vec<V>> {
    paths
        .iter()
        .map(|path| {
            let path = path.as_ref();
            let (name, text) = if path.as_os_str() == STDIN {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .context("reading stdin")?;
                (SourceName::Stdin, text)
            } else {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading `{}`", path.display()))?;
                (SourceName::File(path.to_path_buf()), text)
            };
            format::parse(&text, &name)
        })
        .collect()
}
