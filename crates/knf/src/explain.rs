//! Adds CLI flag `help:` lines to library errors.

use std::path::Path;

use anyhow::anyhow;
use knf::{InterpError, Origin, PathError, ProblemKind};
use knf_convert::LoadError;

/// Adds help to a load error.
///
/// `context` is the `--with` path when one was supplied. Mixed formats then
/// share one format with that file, and a directory at that path is not a
/// config to merge.
pub fn explain_load(err: LoadError, context: Option<&Path>) -> anyhow::Error {
    match &err {
        LoadError::NoInputs => anyhow!("{err}\nhelp: pass --to json or --to toml"),
        LoadError::UnknownExtension { .. } => {
            anyhow!("{err}\nhelp: name the file with a .json or .toml extension")
        }
        LoadError::MixedFormats if context.is_some() => {
            anyhow!("{err}\nhelp: inputs and the --with file must share one format")
        }
        LoadError::MixedFormats => {
            anyhow!("{err}\nhelp: merge JSON and TOML configs separately")
        }
        LoadError::Directory { path } if context.is_some_and(|ctx| ctx == path) => {
            anyhow!("{err}\nhelp: --with takes one file")
        }
        LoadError::Directory { path } => anyhow!(
            "{err}\nhelp: `knf {}/*.toml` merges its files",
            path.display()
        ),
        LoadError::File { .. } => err.into(),
    }
}

/// Adds `-c` help to path errors.
pub fn name_the_inline_config_flag(err: PathError) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => anyhow!(
            "{err}\nhelp: -c takes KEY.PATH=VALUE; an index like servers[0] can be read\n      \
             by a ${{...}} reference but never written — put the value in a file instead"
        ),
        other => other.into(),
    }
}

/// Adds `--interpolate` help to interpolation errors.
///
/// `context` is the `--with` path when one was supplied.
pub fn explain_interp(err: InterpError, context: Option<&Path>) -> anyhow::Error {
    let has_context = context.is_some();
    let mut help = String::new();
    match &err {
        InterpError::Cycle(_) => {
            help.push_str("\nhelp: a reference may not resolve, directly or indirectly, to itself")
        }
        InterpError::Problems(problems) => {
            // One help line per kind present, then the opt-out hint.
            let has = |f: fn(&ProblemKind) -> bool| problems.iter().any(|p| f(&p.kind));
            let syntax = has(|kind| matches!(kind, ProblemKind::Syntax(_)));
            let unresolved = has(|kind| matches!(kind, ProblemKind::Unresolved { .. }));
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
            if has(|kind| matches!(kind, ProblemKind::NotStringifiable { .. })) {
                help.push_str(
                    "\nhelp: an object or array reference must be the whole string, not embedded in one",
                );
            }
            if let Some(path) = context
                && problems.iter().any(|p| p.origin == Origin::Context)
            {
                help.push_str(&format!(
                    "\nhelp: (context) paths are in `{}`",
                    path.display()
                ));
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
