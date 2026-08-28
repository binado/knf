//! Flag names, and where they are allowed to appear.
//!
//! The library crates raise errors that carry key paths, reference spellings
//! and file paths — never a command-line flag, because none of them has heard
//! of one. Every `help:` line in this file exists to close that gap on the way
//! out, and this is the only place in the workspace where `--append`,
//! `--replace`, `--fail`, `--set`, `--input-format`, `--interpolate`, `-f` and
//! `--null-as` appear in an error message.

use anyhow::anyhow;
use knf::{
    IntegerOutOfRange, InterpError, LoadError, MergeError, NonFiniteFloat, NullInToml, PathError,
    Problem, RuleError, RuleErrors, TomlError,
};

/// Adds the command-line spelling to errors produced by the reusable pipeline.
///
/// Every stage passes through here — load, merge, interpolate and emit — so
/// there is one place a library error can pick up a flag name, rather than one
/// per call site. Downcasts rather than matching on a wrapper enum, because the
/// pipeline's errors arrive inside `anyhow` and each typed error is decorated by
/// a different rule. Note the hazard: if `knf-config` ever wraps these in one
/// error type of its own, every downcast below starts missing and nothing here
/// fails to compile — the tests that pin this stderr are what would catch it.
pub fn explain_pipeline(err: anyhow::Error) -> anyhow::Error {
    let err = match err.downcast::<LoadError>() {
        Ok(err) => return explain_load(err),
        Err(err) => err,
    };
    let err = match err.downcast::<MergeError>() {
        Ok(err) => return name_the_flag(err),
        Err(err) => err,
    };
    let err = match err.downcast::<InterpError>() {
        Ok(err) => return explain_interp(err),
        Err(err) => err,
    };
    let err = match err.downcast::<NonFiniteFloat>() {
        Ok(report) => return explain_non_finite(report),
        Err(err) => err,
    };
    // Exhaustive on purpose, rather than the catch-all this used to end with. The
    // dispatch above is by downcast, so a new error type slipping out of the
    // pipeline is already invisible to the compiler; the least this file can do is
    // make a new *variant* of one it already handles a build error instead of a
    // help line that quietly stops appearing.
    match err.downcast::<TomlError>() {
        Ok(TomlError::Null(report)) => explain_null(report),
        Ok(TomlError::Integer(report)) => explain_integer(report),
        // A datetime a caller spelled wrongly while building a `Value` by hand —
        // unreachable from argv, and no flag gets anyone out of it, so there is
        // nothing for this file to add.
        Ok(err @ TomlError::Datetime(_)) => err.into(),
        Err(err) => err,
    }
}

/// Names the two flags that get a document with a null out through TOML.
///
/// `knf-config` reports where the nulls are and stops: emitting JSON instead
/// and substituting a string are both things an *interface* offers, and it has
/// none. The report is deliberately newline-free at the end so this line lands
/// flush against its last `-->`.
fn explain_null(err: NullInToml) -> anyhow::Error {
    anyhow!("{err}\nhelp: emit JSON with -f json, substitute with --null-as, or remove the null")
}

/// Names the flag that emits an integer TOML has no spelling for.
///
/// The other half of [`explain_null`]'s bargain, and the same division of labour:
/// `knf-config` says which keys hold an integer past `i64::MAX`, and the one
/// remedy — emit JSON, where the digits survive exactly — is a flag, so it is
/// ours to name.
fn explain_integer(err: IntegerOutOfRange) -> anyhow::Error {
    anyhow!("{err}\nhelp: TOML integers are signed 64-bit; emit JSON with -f json")
}

/// Names the flag that emits a float JSON has no spelling for.
///
/// The escape runs the other way here: `inf` and `nan` are TOML literals and JSON
/// has no syntax for either, so the format that rescues a null is the one that
/// cannot hold this. Worth stating in the help, since a user who has met the null
/// error will reach for `-f json` by reflex.
fn explain_non_finite(err: NonFiniteFloat) -> anyhow::Error {
    anyhow!("{err}\nhelp: emit TOML with -f toml, which can represent inf and nan")
}

/// Names the flag that resolves an input whose format could not be settled.
///
/// `knf-config` states the problem — stdin has no extension, this file's
/// extension says nothing, this path is a directory — and stops there. The
/// remedy is always a flag or a different argv, so it is always ours.
fn explain_load(err: LoadError) -> anyhow::Error {
    match &err {
        LoadError::StdinNeedsFormat | LoadError::UnknownExtension { .. } => {
            anyhow!("{err}: pass --input-format json or --input-format toml")
        }
        LoadError::Directory { path } => anyhow!(
            "{err}\nhelp: `knf {}/*.toml` merges its files as layers",
            path.display()
        ),
    }
}

/// The established division of labour: `knf-core` renders the path and stays
/// provenance-free, the help line names the flag that carried it.
pub fn name_the_rule_flag(err: PathError, flag: &str) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => {
            anyhow!("{err}\nhelp: {flag} takes a key path; a rule cannot name an array element")
        }
        other => other.into(),
    }
}

/// Same division of labour for `--set`: its paths feed the same conversion.
pub fn name_the_set_flag(err: PathError) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => anyhow!(
            "{err}\nhelp: --set takes KEY.PATH=VALUE; an index like servers[0] can be read\n      \
             by a ${{...}} reference but never written — put the value in a file instead"
        ),
        other => other.into(),
    }
}

/// Turns a rule-set rejection into the flags the user actually typed.
///
/// `knf-core` names strategies, never flags — it has no idea they are spelled
/// `--append`, `--replace` and `--fail` — so the help lines belong here.
pub fn explain_rules(errors: RuleErrors) -> anyhow::Error {
    const FLAGS: &str = "--append, --replace and --fail";
    let mut help = String::new();
    if errors
        .errors()
        .iter()
        .any(|e| matches!(e, RuleError::Conflict { .. }))
    {
        help.push_str(&format!(
            "\nhelp: a path may be named by only one of {FLAGS}"
        ));
    }
    if errors
        .errors()
        .iter()
        .any(|e| matches!(e, RuleError::Unreachable { .. }))
    {
        help.push_str(&format!(
            "\nhelp: {FLAGS} take the whole value at their path, so a rule below one can never fire"
        ));
    }
    anyhow!("{errors}{help}")
}

/// Same division of labour for the errors a rule raises during the merge.
fn name_the_flag(err: MergeError) -> anyhow::Error {
    let help = match err {
        MergeError::Locked { .. } => {
            "help: --fail pins a path to the first layer that sets it; drop the flag or the later value"
        }
        MergeError::AppendKind { .. } => "help: --append needs an array on both sides",
        MergeError::TypeConflict { .. } => return err.into(),
    };
    anyhow!("{err}\n{help}")
}

/// The same division of labour for interpolation.
///
/// `knf-interp` names key paths and reference spellings; it has never heard of
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
