//! `knf <files...>` — merge layers left to right, print one document.
//!
//! Everything argv-shaped lives in this binary: the flag grammar ([`cli`]), the
//! `help:` lines that name a flag ([`explain`]), and the output-format decision
//! below. The pipeline itself is `knf-config`, which knows nothing about any of
//! it.

mod cli;
mod explain;

use std::io::Write;

use anyhow::bail;
use clap::Parser;
use knf::{Format, Map, MergeOpts, PathLeaf, Rules, Strategy, format, load_layers, merge_layers};

use cli::Cli;
use explain::{explain_pipeline, explain_rules, name_the_rule_flag, name_the_set_flag};

fn main() {
    // clap handles --help/--version and exits 2 on usage errors.
    let cli = Cli::parse();

    if let Err(err) = run(cli) {
        // Some errors (the null-in-TOML report, mixed-format, directory) are
        // deliberately multi-line and carry their own `help:` line.
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        std::process::exit(1);
    }
}

/// One pipeline regardless of the formats involved: every layer becomes a
/// `Value`, the fold runs once, and the output format is only consulted at emit.
/// Nothing about JSON or TOML reaches the merge.
fn run(cli: Cli) -> anyhow::Result<()> {
    // Before anything is read: a broken rule set is a mistake in the command
    // line, and saying so must not wait on the files existing or parsing.
    let opts = merge_opts(&cli)?;

    // Between the parse and the fold: the output format is a decision about
    // argv, and the formats it needs are known as soon as the inputs are read.
    // Deciding it after the merge would make a forgotten `-f` queue behind
    // every error in the documents themselves.
    let (layers, input_formats) =
        load_layers(&cli.files, opts.input_format).map_err(explain_pipeline)?;
    let out_format = resolve_output_format(cli.format.map(Format::from), &input_formats)?;

    let merged = merge_layers(layers, opts, &knf::ProcessEnv).map_err(explain_pipeline)?;
    let text = format::emit(merged, out_format, !cli.compact, cli.null_as.as_deref())?;
    write_stdout(&text)
}

/// Builds the merge knobs, validating the whole rule set and every `--set`
/// expression up front.
///
/// Fallible, and called before any input is read: both come from argv alone, so
/// nothing about the files can change whether they are legal.
fn merge_opts(cli: &Cli) -> anyhow::Result<MergeOpts> {
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

    // --set layers are terminal: appended after every file. The RHS parses as
    // JSON with a string fallback, which is `knf-config`'s job. The conversion
    // is also where a bracketed path is rejected, so it runs with the rule set
    // above: up front, not after the files exist or parse.
    let mut overlays: Vec<Map> = Vec::with_capacity(cli.set.len());
    for path_leaf in &cli.set {
        let typed = PathLeaf::<serde_json::Value>::from(path_leaf.clone());
        let json = serde_json::Value::try_from(typed).map_err(name_the_set_flag)?;
        let serde_json::Value::Object(obj) = json else {
            // The expansion nests the leaf under every key in the path, and the
            // grammar rejects an empty path — so it is always an object.
            unreachable!("a --set expression expands to a nested object")
        };
        overlays.push(knf::value::object_from_json(obj));
    }

    Ok(MergeOpts {
        input_format: cli.input_format.map(Format::from),
        strict: cli.strict,
        rules: Rules::build(rules).map_err(explain_rules)?,
        overlays,
        interpolate: cli.interpolate,
    })
}

/// Decides the output format from `-f` and the inputs.
///
/// Following the first input's format would mean reordering arguments silently
/// changes the output encoding, so mixed inputs demand an explicit choice.
fn resolve_output_format(explicit: Option<Format>, inputs: &[Format]) -> anyhow::Result<Format> {
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

/// Writes to stdout, treating a closed pipe as success so `knf big.json | head`
/// does not report an error the user cannot act on.
fn write_stdout(text: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    match std::io::stdout().write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e).context("writing to stdout"),
    }
}
