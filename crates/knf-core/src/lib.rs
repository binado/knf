//! Load, merge and interpolate homogeneous JSON or TOML configuration layers.
//!
//! [`load_layers`] returns native [`Layers`]. Match its variant, then call
//! [`merge`] or [`merge_interpolate`] and [`format::emit`] on that value type.

pub mod format;
pub mod fs;
pub mod glob;
mod inline;
mod interp;
mod merge;
mod path;
pub mod value;

mod env;

use std::path::{Path, PathBuf};

use anyhow::Context;

pub use env::ProcessEnv;
pub use format::{ConfigFormat, Format};
pub use inline::{PathLeaf, json_or_string, toml_or_string};
pub use interp::{
    Cycle, Env, InterpError, InterpOptions, Problem, Syntax, interpolate, interpolate_with_context,
    merge_interpolate,
};
pub use merge::{MergeOptions, merge, merge_into};
pub use path::{PathError, RefPath, Seg, render_path};
pub use value::{ConfigObject, ConfigValue};

/// Why input paths could not be turned into layers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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

/// Native layers, all in one format and in the caller's original order.
#[derive(Debug, Clone, PartialEq)]
pub enum Layers {
    /// JSON documents.
    Json(Vec<serde_json::Value>),
    /// TOML documents.
    Toml(Vec<toml::Value>),
}

/// Resolve a single format from file extensions, without reading any contents.
///
/// Directory inputs are rejected before inference. No inputs default to JSON.
pub fn resolve_format<P: AsRef<Path>>(paths: &[P]) -> Result<Format, LoadError> {
    let mut selected = None;
    for path in paths {
        let path = path.as_ref();
        if path.is_dir() {
            return Err(LoadError::Directory {
                path: path.to_path_buf(),
            });
        }
        let format = Format::from_path(path).ok_or_else(|| LoadError::UnknownExtension {
            path: path.to_path_buf(),
        })?;
        if selected.is_some_and(|previous| previous != format) {
            return Err(LoadError::MixedFormats);
        }
        selected = Some(format);
    }
    Ok(selected.unwrap_or(Format::Json))
}

/// Read homogeneous native layers after resolving one format for the entire list.
///
/// Mixed formats fail before any document contents are read. Empty input
/// returns JSON layers.
pub fn load_layers<P: AsRef<Path>>(paths: &[P]) -> anyhow::Result<Layers> {
    match resolve_format(paths)? {
        Format::Json => Ok(Layers::Json(load_native(paths)?)),
        Format::Toml => Ok(Layers::Toml(load_native(paths)?)),
    }
}

fn load_native<P: AsRef<Path>, V: ConfigFormat>(paths: &[P]) -> anyhow::Result<Vec<V>> {
    paths
        .iter()
        .map(|path| {
            let path = path.as_ref();
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading `{}`", path.display()))?;
            format::parse(&text, path)
        })
        .collect()
}
