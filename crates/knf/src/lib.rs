//! Load and merge layered JSON and TOML configuration files.

#[cfg(feature = "cli")]
pub mod cli;
pub mod format;
pub mod interp;
pub mod value;

use std::io::Read;
#[cfg(feature = "cli")]
use std::io::Write;
use std::path::Path;

#[cfg(feature = "cli")]
use anyhow::anyhow;
use anyhow::{Context, bail};
use knf_core::{MergeOptions as CoreMergeOptions, merge_with};
#[cfg(feature = "cli")]
use knf_core::{RuleError, RuleErrors};
#[cfg(feature = "cli")]
use knf_dotted::{PathError, PathLeaf};
#[cfg(feature = "cli")]
use knf_interp::{InterpError, Problem};

/// The core types this crate's own signatures are written in. A consumer of
/// `knf-cli --no-default-features` depends on this crate alone, so `merge`'s
/// result, `MergeOpts`' fields and the error it hands back must all be
/// nameable from here.
pub use knf_core::{Map, MergeError, Rules, Strategy, Value};

#[cfg(feature = "cli")]
use cli::Cli;
use format::{Format, SourceName};
use interp::ProcessEnv;

/// The positional that means "read stdin".
const STDIN: &str = "-";

/// Options for loading and merging configuration files.
///
/// The files named by [`merge`] are followed by `overlays`, all in one flat,
/// strictly-left fold. This makes an overlay supplied by another interface
/// (for example the CLI's `--set`, or a future language binding) behave exactly
/// like one more terminal file layer.
#[derive(Debug, Clone, PartialEq)]
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
    /// Resolve document and process-environment references after merging.
    pub interpolate: bool,
}

impl Default for MergeOpts {
    fn default() -> Self {
        Self {
            input_format: None,
            strict: false,
            rules: Rules::EMPTY,
            overlays: Vec::new(),
            interpolate: false,
        }
    }
}

/// Loads `paths`, parses every file into the common IR, and merges the flat
/// layer list from left to right.
///
/// JSON and TOML may be mixed. Their formats are inferred from file extensions
/// unless [`MergeOpts::input_format`] overrides inference. A path equal to `-`
/// reads standard input and therefore requires an explicit input format.
///
/// Interpolation, when enabled, runs once on the merged document. Environment
/// values come from the current process and follow the same JSON-or-string
/// typing rule as CLI `--set` values.
pub fn merge<P: AsRef<Path>>(paths: &[P], opts: MergeOpts) -> anyhow::Result<Value> {
    let (layers, _formats) = load_layers(paths, opts.input_format)?;
    merge_layers(layers, opts)
}

/// Reads and parses every positional into the merge IR, keeping the format each
/// input was read as.
///
/// Separate from the fold because the CLI resolves the *output* format in
/// between, and the observed formats are the input to that decision. A missing
/// `-f` is a mistake in argv alone: reporting it must not wait behind a merge
/// conflict the user would otherwise fix first, only to learn about the flag on
/// the next run. The formats are a presentation concern and so are absent from
/// the public merge result.
fn load_layers<P: AsRef<Path>>(
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
fn merge_layers(mut layers: Vec<Value>, opts: MergeOpts) -> anyhow::Result<Value> {
    layers.reserve(opts.overlays.len());
    layers.extend(opts.overlays.into_iter().map(Value::Object));

    let core_opts = CoreMergeOptions {
        strict: opts.strict,
        rules: opts.rules,
    };
    let merged = merge_with(layers, &core_opts)?;
    // After the merge, before the emit, and never per layer: a reference reads
    // the document the user is actually going to get. `--set` layers therefore
    // interpolate like any other layer, and `--strict` has already run — it
    // compares the types values had when they were *written*, so a `"${port}"`
    // was a string when it looked.
    let merged = if opts.interpolate {
        knf_interp::interpolate(merged, &ProcessEnv)?
    } else {
        merged
    };
    Ok(merged)
}

/// `knf <files...>` — merge layers left to right, print one document.
///
/// One pipeline regardless of the formats involved: every layer becomes a
/// [`Value`], the fold runs once, and the output format is only consulted at
/// emit. Nothing about JSON or TOML reaches the merge.
#[cfg(feature = "cli")]
pub fn run(cli: Cli) -> anyhow::Result<()> {
    // Before anything is read: a broken rule set is a mistake in the command
    // line, and saying so must not wait on the files existing or parsing.
    let core_opts = merge_options(&cli)?;

    // --set layers are terminal: appended after every file. The RHS parses as
    // JSON with a string fallback, which is knf-dotted's job. The conversion
    // is also where a bracketed path is rejected, so it runs with the rule
    // set above: up front, not after the files exist or parse.
    let mut set_layers: Vec<Map> = Vec::with_capacity(cli.set.len());
    for path_leaf in &cli.set {
        let typed = PathLeaf::<serde_json::Value>::from(path_leaf.clone());
        let json = serde_json::Value::try_from(typed).map_err(name_the_set_flag)?;
        let serde_json::Value::Object(obj) = json else {
            // The expansion nests the leaf under every key in the path, and
            // the grammar rejects an empty path — so it is always an object.
            unreachable!("a --set expression expands to a nested object")
        };
        set_layers.push(value::object_from_json(obj));
    }

    let opts = MergeOpts {
        input_format: cli.input_format,
        strict: core_opts.strict,
        rules: core_opts.rules,
        overlays: set_layers,
        interpolate: cli.interpolate,
    };

    // Between the parse and the fold: the output format is a decision about
    // argv, and the formats it needs are known as soon as the inputs are read.
    // Deciding it after the merge would make a forgotten `-f` queue behind
    // every error in the documents themselves.
    let (layers, input_formats) = load_layers(&cli.files, opts.input_format)?;
    let out_format = resolve_output_format(cli.format, &input_formats)?;

    let merged = merge_layers(layers, opts).map_err(explain_pipeline)?;
    let text = format::emit(merged, out_format, !cli.compact, cli.null_as.as_deref())?;
    write_stdout(&text)
}

/// Adds the command-line spelling to errors produced by the reusable pipeline.
#[cfg(feature = "cli")]
fn explain_pipeline(err: anyhow::Error) -> anyhow::Error {
    let err = match err.downcast::<MergeError>() {
        Ok(err) => return name_the_flag(err),
        Err(err) => err,
    };
    match err.downcast::<InterpError>() {
        Ok(err) => explain_interp(err),
        Err(err) => err,
    }
}

/// Reads one positional, resolving its format.
fn read_input(
    path: &Path,
    override_format: Option<Format>,
) -> anyhow::Result<(SourceName, Format, String)> {
    if path.as_os_str() == STDIN {
        let format = override_format.context(
            "`-` reads stdin, which has no extension: pass --input-format json or --input-format toml",
        )?;
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading stdin")?;
        return Ok((SourceName::Stdin, format, text));
    }

    if path.is_dir() {
        bail!(
            "`{}` is a directory; knf takes files as layers\n\
             help: `knf {}/*.toml` merges its files as layers",
            path.display(),
            path.display(),
        );
    }

    let format = match override_format {
        Some(format) => format,
        None => Format::from_path(path).with_context(|| {
            format!(
                "cannot infer a format from `{}`: pass --input-format json or --input-format toml",
                path.display()
            )
        })?,
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading `{}`", path.display()))?;
    Ok((SourceName::File(path.to_path_buf()), format, text))
}

/// Decides the output format from `-f` and the inputs.
///
/// Following the first input's format would mean reordering arguments silently
/// changes the output encoding, so mixed inputs demand an explicit choice.
pub fn resolve_output_format(
    explicit: Option<Format>,
    inputs: &[Format],
) -> anyhow::Result<Format> {
    if let Some(format) = explicit {
        return Ok(format);
    }
    let mut distinct: Vec<Format> = Vec::new();
    for format in inputs {
        if !distinct.contains(format) {
            distinct.push(*format);
        }
    }
    match distinct.as_slice() {
        // No file inputs at all — `knf --set a.b=1`.
        [] => Ok(Format::Json),
        [only] => Ok(*only),
        mixed => {
            let names: Vec<String> = mixed.iter().map(Format::to_string).collect();
            bail!(
                "inputs mix {} formats; -f is required to choose the output format\n\
                 help: pass -f json or -f toml",
                names.join(" and "),
            )
        }
    }
}

/// Builds the merge knobs, validating the whole rule set up front.
///
/// Fallible, and called before any input is read: the rules come from argv
/// alone, so nothing about the files can change whether they are legal.
#[cfg(feature = "cli")]
pub fn merge_options(cli: &Cli) -> anyhow::Result<CoreMergeOptions> {
    let flags = [
        ("--append", &cli.append, Strategy::Append),
        ("--replace", &cli.replace, Strategy::Replace),
        ("--fail", &cli.fail, Strategy::Fail),
    ];
    let mut rules: Vec<(Vec<String>, Strategy)> = Vec::new();
    for (flag, paths, strategy) in flags {
        for path in paths {
            // The one write-side predicate, run per flag so the error can
            // name it: rules name keys, never array elements.
            let keys = path
                .clone()
                .try_into_keys()
                .map_err(|err| name_the_rule_flag(err, flag))?;
            rules.push((keys, strategy));
        }
    }

    Ok(CoreMergeOptions {
        strict: cli.strict,
        rules: Rules::build(rules).map_err(explain_rules)?,
    })
}

/// The established division of labour: `knf-dotted` renders the path and
/// stays provenance-free, the help line names the flag that carried it.
#[cfg(feature = "cli")]
fn name_the_rule_flag(err: PathError, flag: &str) -> anyhow::Error {
    match err {
        PathError::IndexInKeyPath { .. } => {
            anyhow!("{err}\nhelp: {flag} takes a key path; a rule cannot name an array element")
        }
        other => other.into(),
    }
}

/// Same division of labour for `--set`: its paths feed the same conversion.
#[cfg(feature = "cli")]
fn name_the_set_flag(err: PathError) -> anyhow::Error {
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
#[cfg(feature = "cli")]
fn explain_rules(errors: RuleErrors) -> anyhow::Error {
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
#[cfg(feature = "cli")]
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
#[cfg(feature = "cli")]
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

/// Writes to stdout, treating a closed pipe as success so `knf big.json | head`
/// does not report an error the user cannot act on.
#[cfg(feature = "cli")]
pub fn write_stdout(text: &str) -> anyhow::Result<()> {
    match std::io::stdout().write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e).context("writing to stdout"),
    }
}
