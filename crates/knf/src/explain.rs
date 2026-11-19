//! Adds CLI flag `help:` lines to library errors.

use std::path::Path;

use anyhow::anyhow;
use knf::{InterpError, LoadError, PathError, Problem};

/// Adds CLI help to unwrapped `LoadError` or `InterpError` pipeline errors.
///
/// `context` is the `--with` path for load failures (`None` for later stages).
/// Uses `anyhow::Error::downcast`, so wrapped errors pass through unchanged.
pub fn explain_pipeline(err: impl Into<anyhow::Error>, context: Option<&Path>) -> anyhow::Error {
    let err = err.into();
    let err = match err.downcast::<LoadError>() {
        Ok(err) => return explain_load(err, context),
        Err(err) => err,
    };
    match err.downcast::<InterpError>() {
        Ok(err) => explain_interp(err, false),
        Err(err) => err,
    }
}

/// Adds help to a load error.
///
/// `context` is the `--with` path when one was supplied. Mixed formats then
/// share one format with that file, and a directory at that path is not a
/// merge layer.
fn explain_load(err: LoadError, context: Option<&Path>) -> anyhow::Error {
    match &err {
        LoadError::UnknownExtension { .. } => {
            anyhow!("{err}\nhelp: name the file with a .json or .toml extension")
        }
        LoadError::MixedFormats if context.is_some() => {
            anyhow!("{err}\nhelp: inputs and the --with file must share one format")
        }
        LoadError::MixedFormats => {
            anyhow!("{err}\nhelp: merge JSON layers and TOML layers separately")
        }
        LoadError::Directory { path } if context.is_some_and(|ctx| ctx == path) => {
            anyhow!("{err}\nhelp: --with takes one file")
        }
        LoadError::Directory { path } => anyhow!(
            "{err}\nhelp: `knf {}/*.toml` merges its files as layers",
            path.display()
        ),
    }
}

/// Adds `-c` help to path errors.
pub fn name_the_inline_layer_flag(err: PathError) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => anyhow!(
            "{err}\nhelp: -c takes KEY.PATH=VALUE; an index like servers[0] can be read\n      \
             by a ${{...}} reference but never written — put the value in a file instead"
        ),
        other => other.into(),
    }
}

/// Adds `--interpolate` help to interpolation errors.
pub fn explain_interp(err: InterpError, has_context: bool) -> anyhow::Error {
    let mut help = String::new();
    match &err {
        InterpError::Cycle(_) => {
            help.push_str("\nhelp: a reference may not resolve, directly or indirectly, to itself")
        }
        InterpError::Problems(problems) => {
            // One help line per kind present, then the opt-out hint.
            let has = |f: fn(&Problem) -> bool| problems.iter().any(f);
            let syntax = has(|p| matches!(p, Problem::Syntax { .. }));
            let unresolved = has(|p| matches!(p, Problem::Unresolved { .. }));
            if syntax {
                help.push_str(
                    "\nhelp: a reference is `${key.path}` (with `[n]` for array elements) or `${env:NAME}`; write `$$` for a literal `$`",
                );
            }
            if unresolved {
                if has_context {
                    help.push_str(
                        "\nhelp: `${key.path}` names a key in the merged document or --with context, `${env:NAME}` an environment variable",
                    );
                } else {
                    help.push_str(
                    "\nhelp: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable",
                    );
                }
            }
            if has(|p| matches!(p, Problem::NotStringifiable { .. })) {
                help.push_str(
                    "\nhelp: an object or array reference must be the whole string, not embedded in one",
                );
            }
            if syntax || unresolved {
                if has_context {
                    help.push_str(
                        "\nhelp: drop --interpolate and --with to pass `${...}` through untouched",
                    );
                } else {
                    help.push_str("\nhelp: drop --interpolate to pass `${...}` through untouched");
                }
            }
        }
    }
    anyhow!("{err}{help}")
}
