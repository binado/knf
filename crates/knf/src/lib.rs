//! The `knf` pipeline: load layers, merge, emit.

pub mod cli;
pub mod format;
pub mod value;

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, anyhow, bail};
use knf_core::{
    MergeError, MergeOptions, RuleError, RuleErrors, Rules, Strategy, Value, merge_with,
};
use knf_dotted::PathLeaf;

use cli::Cli;
use format::{Format, SourceName};

/// The positional that means "read stdin".
const STDIN: &str = "-";

/// `knf <files...>` — merge layers left to right, print one document.
///
/// One pipeline regardless of the formats involved: every layer becomes a
/// [`Value`], the fold runs once, and the output format is only consulted at
/// emit. Nothing about JSON or TOML reaches the merge.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    // Before anything is read: a broken rule set is a mistake in the command
    // line, and saying so must not wait on the files existing or parsing.
    let opts = merge_options(&cli)?;

    let mut layers: Vec<Value> = Vec::new();
    let mut input_formats: Vec<Format> = Vec::new();

    for path in &cli.files {
        let (name, format, text) = read_input(path, cli.input_format)?;
        let value = format::parse(format, &text, &name)?;
        input_formats.push(format);
        layers.push(value);
    }

    // --set layers are terminal: appended after every file. The RHS parses as
    // JSON with a string fallback, which is knf-dotted's job.
    for path_leaf in &cli.set {
        let json = serde_json::Value::from(PathLeaf::<serde_json::Value>::from(path_leaf.clone()));
        layers.push(value::from_json(json));
    }

    let out_format = resolve_output_format(cli.format, &input_formats)?;

    let merged = merge_with(layers, &opts).map_err(name_the_flag)?;
    let text = format::emit(
        merged,
        out_format,
        !cli.compact,
        cli.null_placeholder.as_deref(),
    )?;
    write_stdout(&text)
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
pub fn merge_options(cli: &Cli) -> anyhow::Result<MergeOptions> {
    let flags = [
        (&cli.append, Strategy::Append),
        (&cli.replace, Strategy::Replace),
        (&cli.fail, Strategy::Fail),
    ];
    let rules: Vec<(Vec<String>, Strategy)> = flags
        .into_iter()
        .flat_map(|(paths, strategy)| {
            paths
                .iter()
                .map(move |path| (path.segments().to_vec(), strategy))
        })
        .collect();

    Ok(MergeOptions {
        strict: cli.strict,
        rules: Rules::build(rules).map_err(explain_rules)?,
    })
}

/// Turns a rule-set rejection into the flags the user actually typed.
///
/// `knf-core` names strategies, never flags — it has no idea they are spelled
/// `--append`, `--replace` and `--fail` — so the help lines belong here.
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

/// Writes to stdout, treating a closed pipe as success so `knf big.json | head`
/// does not report an error the user cannot act on.
pub fn write_stdout(text: &str) -> anyhow::Result<()> {
    match std::io::stdout().write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e).context("writing to stdout"),
    }
}
