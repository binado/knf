//! Load and merge layered JSON and TOML configuration files.
//!
//! The pipeline in one place: read each positional into [`Value`], fold the flat
//! layer list from left to right, resolve `${...}` references once at the end.
//! [`merge`] is the whole of it for a caller that wants a document back;
//! [`load_layers`] and [`merge_layers`] are the same two halves apart, for a
//! caller that has a decision to make in between — which is what `knf-cli` does
//! with the output format.
//!
//! Formats appear here and nowhere else. [`knf_core`] never learns that JSON or
//! TOML exist, and [`knf_interp`] runs on the merged [`Value`] without either.
//!
//! No flag names. An error from this crate carries key paths and file paths;
//! the command-line spelling that produced it is `knf-cli`'s to add, the same
//! division of labour [`knf_core`] and [`knf_interp`] already keep. That is what
//! [`LoadError`] exists for — the three failures that used to say
//! `--input-format` are typed instead, so the caller decides how to name them.

pub mod format;
mod set;
pub mod value;

mod env;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;
use knf_core::{MergeOptions as CoreMergeOptions, merge_with};

/// The core and interpolation types this crate's own signatures are written in.
///
/// A consumer of `knf-config` depends on this crate alone, so [`merge`]'s
/// result, [`MergeOpts`]' fields, the [`Env`] it can be handed and every error
/// it returns must all be nameable from here.
pub use knf_core::{
    Map, MergeError, PathError, RefPath, RuleError, RuleErrors, Rules, Seg, Strategy, Value,
};
pub use knf_interp::{Env, EnvValue, InterpError, Problem};

pub use env::ProcessEnv;
pub use format::Format;
pub use set::{PathLeaf, json_or_string};

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

/// Options for loading and merging configuration files.
///
/// The files named by [`merge`] are followed by `overlays`, all in one flat,
/// strictly-left fold. This makes an overlay supplied by another interface
/// (for example the CLI's `--set`, or a language binding) behave exactly like
/// one more terminal file layer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeOpts {
    /// Treat every input path as this format instead of inferring extensions.
    pub input_format: Option<Format>,
    /// Error when a layer changes the kind of an existing key.
    pub strict: bool,
    /// Per-path overrides of the default merge strategy.
    pub rules: Rules,
    /// In-memory layers appended after every file, in order.
    ///
    /// Maps rather than [`Value`]s for the reason [`format::parse`] requires an
    /// object at the top level: a scalar layer does not shadow a key, it
    /// replaces the whole document with something no format can emit.
    pub overlays: Vec<Map>,
    /// Resolve document and environment references after merging.
    pub interpolate: bool,
}

/// Loads `paths`, parses every file into the common IR, and merges the flat
/// layer list from left to right.
///
/// JSON and TOML may be mixed. Their formats are inferred from file extensions
/// unless [`MergeOpts::input_format`] overrides inference. A path equal to
/// [`STDIN`] reads standard input and therefore requires an explicit input
/// format.
///
/// Interpolation, when enabled, runs once on the merged document and reads the
/// process environment. Use [`merge_with_env`] to supply the environment
/// instead — the output is then a function of the inputs alone.
pub fn merge<P: AsRef<Path>>(paths: &[P], opts: MergeOpts) -> anyhow::Result<Value> {
    merge_with_env(paths, opts, &ProcessEnv)
}

/// [`merge`], with the environment `${env:NAME}` resolves against supplied by
/// the caller.
///
/// `knf-interp` takes its environment through a trait precisely so a caller can
/// decide what "the environment" is; hardcoding the process would make the one
/// public entry point offering interpolation the only one that cannot use that
/// seam. `env` is ignored entirely unless [`MergeOpts::interpolate`] is set.
pub fn merge_with_env<P: AsRef<Path>>(
    paths: &[P],
    opts: MergeOpts,
    env: &dyn Env,
) -> anyhow::Result<Value> {
    let (layers, _formats) = load_layers(paths, opts.input_format)?;
    merge_layers(layers, opts, env)
}

/// Reads and parses every positional into the merge IR, keeping the format each
/// input was read as.
///
/// Separate from the fold because a caller may have a decision to make in
/// between, and the observed formats are the input to it: `knf-cli` resolves the
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

/// Appends the overlays to the file layers and folds the flat list, resolving
/// references once at the end when asked.
pub fn merge_layers(
    mut layers: Vec<Value>,
    opts: MergeOpts,
    env: &dyn Env,
) -> anyhow::Result<Value> {
    layers.reserve(opts.overlays.len());
    layers.extend(opts.overlays.into_iter().map(Value::Object));

    let core_opts = CoreMergeOptions {
        strict: opts.strict,
        rules: opts.rules,
    };
    let merged = merge_with(layers, &core_opts)?;
    // After the merge, before the emit, and never per layer: a reference reads
    // the document the caller is actually going to get. Overlays therefore
    // interpolate like any other layer, and strict mode has already run — it
    // compares the types values had when they were *written*, so a `"${port}"`
    // was a string when it looked.
    let merged = if opts.interpolate {
        knf_interp::interpolate(merged, env)?
    } else {
        merged
    };
    Ok(merged)
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
