//! Load, merge and emit layered JSON and TOML configuration.
//!
//! Three steps, each a function, and a caller composes them:
//!
//! 1. [`load_layers`] reads each path into a [`Value`], keeping the format it
//!    was read as;
//! 2. [`merge`] folds the flat layer list from left to right — any in-memory
//!    overlays are just more layers appended to the list;
//! 3. [`interpolate`], if wanted, resolves `${...}` references once, on the
//!    merged document.
//!
//! [`format::emit`] renders the result. JSON and TOML appear only in
//! [`format`](mod@format) and [`value`]; the merge and interpolation never learn either
//! exists.
//!
//! No flag names. An error from this crate carries key paths and file paths;
//! the command-line spelling that produced it is `knf-cli`'s to add. That is
//! what [`LoadError`] exists for — the failures that have an obvious
//! command-line remedy are typed, so the caller decides how to name them.

pub mod format;
mod interp;
mod ir;
mod merge;
mod path;
mod set;
pub mod value;

mod env;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub use env::ProcessEnv;
pub use format::Format;
pub use interp::{Cycle, Env, EnvValue, InterpError, Problem, Syntax, interpolate};
pub use ir::{Map, Number, Value};
pub use merge::{MergeError, MergeOptions, merge, merge_into};
pub use path::{PathError, RefPath, Seg, render_path};
pub use set::{PathLeaf, json_or_string};
pub use value::{BadDatetime, IntegerOutOfRange, NonFiniteFloat, NullInToml, TomlError};

use format::SourceName;

/// The positional that means "read stdin".
pub const STDIN: &str = "-";

/// Why a positional could not be turned into a layer.
///
/// Carries paths and nothing else. Each of these has an obvious command-line
/// remedy and no library-level one, which is exactly why the remedy is not
/// spelled here: `knf-cli` matches on the variant and adds the flag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// `-` was given without an explicit input format.
    #[error("`-` reads stdin, which has no extension")]
    StdinNeedsFormat,
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
    /// The path this failed on, or `None` for stdin.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::StdinNeedsFormat => None,
            Self::Directory { path } | Self::UnknownExtension { path } => Some(path),
        }
    }
}

/// Reads and parses every positional into the merge IR, keeping the format each
/// input was read as.
///
/// A path equal to [`STDIN`] reads standard input and therefore requires an
/// explicit `input_format`; otherwise the format is inferred from the extension
/// unless `input_format` overrides it. JSON and TOML may be mixed.
///
/// The formats are returned because a caller may have a decision to make before
/// the fold, and they are the input to it: `knf-cli` resolves the
/// *output* format here. A missing `-f` is a mistake in argv alone, and
/// reporting it must not wait behind a merge conflict the user would otherwise
/// fix first, only to learn about the flag on the next run.
pub fn load_layers<P: AsRef<Path>>(
    paths: &[P],
    input_format: Option<Format>,
) -> anyhow::Result<(Vec<Value>, Vec<Format>)> {
    let mut layers: Vec<Value> = Vec::with_capacity(paths.len());
    let mut input_formats: Vec<Format> = Vec::with_capacity(paths.len());

    for path in paths {
        let (name, format, text) = read_input(path.as_ref(), input_format)?;
        let value = format::parse(format, &text, &name)?;
        input_formats.push(format);
        layers.push(value);
    }
    Ok((layers, input_formats))
}

/// Reads one positional, resolving its format.
fn read_input(
    path: &Path,
    override_format: Option<Format>,
) -> anyhow::Result<(SourceName, Format, String)> {
    if path.as_os_str() == STDIN {
        let format = override_format.ok_or(LoadError::StdinNeedsFormat)?;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        return Ok((SourceName::Stdin, format, text));
    }

    if path.is_dir() {
        return Err(LoadError::Directory {
            path: path.to_path_buf(),
        }
        .into());
    }

    let format = match override_format {
        Some(format) => format,
        None => Format::from_path(path).ok_or_else(|| LoadError::UnknownExtension {
            path: path.to_path_buf(),
        })?,
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading `{}`", path.display()))?;
    Ok((SourceName::File(path.to_path_buf()), format, text))
}
