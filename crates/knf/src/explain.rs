//! Adds `help:` lines naming CLI flags to library errors. The only place flag
//! names appear in error messages.

use std::path::Path;

use anyhow::anyhow;
use knf::fs::{AccumulateError, AccumulateTargetError};
use knf::{InterpError, LoadError, MergeError, PathError, Problem};

pub fn context_stdin_conflict() -> &'static str {
    "stdin cannot supply both a merge layer and --with context"
}

/// Preserve the command-line vocabulary for target validation.
pub fn explain_accumulate_target(err: AccumulateTargetError) -> String {
    match err {
        AccumulateTargetError::Stdin => "--accumulate does not accept stdin",
        AccumulateTargetError::InvalidPath => {
            "--accumulate requires a relative target path without .. components"
        }
        AccumulateTargetError::UnknownExtension => {
            "--accumulate requires a target with a JSON or TOML extension"
        }
    }
    .to_owned()
}

/// Renders discovery errors with their paths and I/O context.
pub fn explain_accumulate(err: AccumulateError) -> anyhow::Error {
    match err {
        AccumulateError::Inspect { path, source } => {
            anyhow::Error::new(source).context(format!("inspecting `{}`", path.display()))
        }
        AccumulateError::List { path, source } => {
            anyhow::Error::new(source).context(format!("listing `{}`", path.display()))
        }
        AccumulateError::Directory { path } | AccumulateError::NonRegular { path } => {
            anyhow!("`{}` is not a regular file", path.display())
        }
        AccumulateError::CurrentDirectory(source) => {
            anyhow::Error::new(source).context("determining the working directory")
        }
    }
}

/// Adds CLI help to a pipeline error by downcasting it.
///
/// `context` is the `--with` path for load failures, and `None` for later
/// stages. If `knf-core` starts wrapping these errors, the downcasts silently
/// miss; the stderr snapshot tests catch that.
pub fn explain_pipeline(err: impl Into<anyhow::Error>, context: Option<&Path>) -> anyhow::Error {
    let err = err.into();
    let err = match err.downcast::<LoadError>() {
        Ok(err) => return explain_load(err, context),
        Err(err) => err,
    };
    let err = match err.downcast::<MergeError>() {
        // A type conflict is fixed in the documents, not on the command line.
        // Match variants exhaustively so new merge failures require a decision.
        Ok(err @ MergeError::TypeConflict { .. }) => return err.into(),
        Err(err) => err,
    };
    match err.downcast::<InterpError>() {
        Ok(err) => explain_interp(err, false, false),
        Err(err) => err,
    }
}

/// Names the flag that resolves a format-selection error.
///
/// `context` is the `--with` path when one was supplied. Mixed formats then
/// share one format with that file, and a directory at that path is not a
/// merge layer.
fn explain_load(err: LoadError, context: Option<&Path>) -> anyhow::Error {
    match &err {
        LoadError::StdinNeedsFormat | LoadError::UnknownExtension { .. } => {
            anyhow!("{err}: pass -f json or -f toml")
        }
        LoadError::MixedFormats if context.is_some() => anyhow!(
            "{err}\nhelp: inputs and the --with file must share one format; pass -f json or -f toml"
        ),
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
pub fn explain_interp(err: InterpError, has_context: bool, has_merge_key: bool) -> anyhow::Error {
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
            if has(|p| {
                matches!(
                    p,
                    Problem::MergeReference { .. } | Problem::MergeObject { .. }
                )
            }) {
                help.push_str(
                    "\nhelp: --merge-key selects a literal key whose value must be one whole-string reference to an object/table",
                );
            }
            if syntax || unresolved {
                if has_merge_key && has_context {
                    help.push_str("\nhelp: drop --interpolate, --merge-key and --with to pass `${...}` through untouched");
                } else if has_merge_key {
                    help.push_str("\nhelp: drop --interpolate and --merge-key to pass `${...}` through untouched");
                } else if has_context {
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
