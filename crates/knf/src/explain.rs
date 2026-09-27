//! Flag names, and where they are allowed to appear.
//!
//! The library crates raise errors that carry key paths, reference spellings
//! and file paths — never a command-line flag, because none of them has heard
//! of one. Every `help:` line in this file exists to close that gap on the way
//! out, and this is the only place in the workspace where `--set`,
//! `--interpolate` and `-f` appear in an error
//! message.

use anyhow::anyhow;
use knf::fs::{AccumulateError, AccumulateTargetError};
use knf::{InterpError, LoadError, MergeError, PathError, Problem};

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

/// Render discovery paths and I/O context exactly as the CLI did before extraction.
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

/// Adds the command-line spelling to errors produced by the reusable pipeline.
///
/// Every stage passes through here — load, merge, interpolate and emit — so
/// there is one place a library error can pick up a flag name, rather than one
/// per call site. Downcasts rather than matching on a wrapper enum, because the
/// pipeline's errors arrive inside `anyhow` and each typed error is decorated by
/// a different rule. Note the hazard: if `knf-core` ever wraps these in one
/// error type of its own, every downcast below starts missing and nothing here
/// fails to compile — the tests that pin this stderr are what would catch it.
pub fn explain_pipeline(err: impl Into<anyhow::Error>) -> anyhow::Error {
    let err = err.into();
    let err = match err.downcast::<LoadError>() {
        Ok(err) => return explain_load(err),
        Err(err) => err,
    };
    let err = match err.downcast::<MergeError>() {
        // A type conflict is fixed in the documents, not on the command line.
        // Match variants exhaustively so new merge failures require a decision.
        Ok(err @ MergeError::TypeConflict { .. }) => return err.into(),
        Err(err) => err,
    };
    match err.downcast::<InterpError>() {
        Ok(err) => explain_interp(err),
        Err(err) => err,
    }
}

/// Names the flag that resolves an input whose format could not be settled.
///
/// `knf-core` states the problem — stdin has no extension, this file's
/// extension says nothing, this path is a directory — and stops there. The
/// remedy is always a flag or a different argv, so it is always ours.
fn explain_load(err: LoadError) -> anyhow::Error {
    match &err {
        LoadError::StdinNeedsFormat | LoadError::UnknownExtension { .. } => {
            anyhow!("{err}: pass -f json or -f toml")
        }
        LoadError::MixedFormats => {
            anyhow!("{err}\nhelp: merge JSON layers and TOML layers separately")
        }
        LoadError::Directory { path } => anyhow!(
            "{err}\nhelp: `knf {}/*.toml` merges its files as layers",
            path.display()
        ),
    }
}

/// The established division of labour: `knf-core` renders the path and stays
/// provenance-free, the help line names the flag that carried it.
pub fn name_the_set_flag(err: PathError) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => anyhow!(
            "{err}\nhelp: --set takes KEY.PATH=VALUE; an index like servers[0] can be read\n      \
             by a ${{...}} reference but never written — put the value in a file instead"
        ),
        other => other.into(),
    }
}

/// The `--shallow` counterpart of [`name_the_set_flag`]: the path error names
/// the path, and only this layer knows which flag it came from.
pub fn name_the_shallow_flag(err: PathError) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => anyhow!(
            "{err}\nhelp: --shallow takes a KEY.PATH to an object; an index like servers[0]\n      \
             names an array element, and arrays are never merged into"
        ),
        other => other.into(),
    }
}

/// The same division of labour for interpolation.
///
/// Interpolation names key paths and reference spellings; it has never heard of
/// `--interpolate`, so the flag only appears here.
fn explain_interp(err: InterpError) -> anyhow::Error {
    let mut help = String::new();
    match &err {
        InterpError::Cycle(_) => {
            help.push_str("\nhelp: a reference may not resolve, directly or indirectly, to itself")
        }
        InterpError::Problems(problems) => {
            // One help line per kind present, in the order the message lists
            // them, then the escape that applies to a document whose `${...}`
            // was never meant for knf in the first place.
            let has = |f: fn(&Problem) -> bool| problems.iter().any(f);
            let syntax = has(|p| matches!(p, Problem::Syntax { .. }));
            let unresolved = has(|p| matches!(p, Problem::Unresolved { .. }));
            if syntax {
                help.push_str(
                    "\nhelp: a reference is `${key.path}` (with `[n]` for array elements) or `${env:NAME}`; write `$$` for a literal `$`",
                );
            }
            if unresolved {
                help.push_str(
                    "\nhelp: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable",
                );
            }
            if has(|p| matches!(p, Problem::NotStringifiable { .. })) {
                help.push_str(
                    "\nhelp: an object or array reference must be the whole string, not embedded in one",
                );
            }
            if syntax || unresolved {
                help.push_str("\nhelp: drop --interpolate to pass `${...}` through untouched");
            }
        }
    }
    anyhow!("{err}{help}")
}
