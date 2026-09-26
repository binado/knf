//! `knf <files...>` — merge layers left to right, print one document.
//!
//! Everything argv-shaped lives in this binary: the flag grammar ([`cli`]), the
//! `help:` lines that name a flag ([`explain`]), and the output-format decision
//! below. The pipeline itself is `knf-core`, which knows nothing about any of
//! it.

mod cascade;
mod cli;
mod explain;

use std::io::Write;
use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;
use knf::{
    Format, MergeOptions, PathLeaf, ProcessEnv, Value, format, interpolate, load_layers, merge,
};

use cli::Cli;
use explain::{explain_pipeline, name_the_set_flag};

// Entry point for the `knf-cli` binary. `knf-py` includes this file and calls
// `main_from` instead, so the function is unused in that compilation.
#[allow(dead_code)]
fn main() {
    // `std::env::args` is right for the `knf-cli` binary. The pyknf wheel's
    // command is a Python script, whose process argv starts with the interpreter,
    // so that caller passes `sys.argv` to `main_from` instead.
    main_from(std::env::args_os());
}

/// Runs the command line over `args`, whose first item is the program name.
pub fn main_from<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    // clap handles --help/--version and exits 2 on usage errors.
    let cli = Cli::parse_from(args);
    if let Err(err) = cli.validate() {
        err.exit();
    }

    if let Err(err) = run(cli) {
        // Some errors are deliberately multi-line: the null-in-TOML report and
        // the directory hint get their `help:` line from `explain`, the
        // mixed-format one carries its own from below.
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        std::process::exit(1);
    }
}

/// Prepare explicit file inputs and terminal overlays for the shared pipeline.
fn run(cli: Cli) -> anyhow::Result<()> {
    // Before anything is read: a malformed --set is a mistake in the command
    // line, and saying so must not wait on the files existing or parsing.
    let overlays = overlays(&cli)?;

    let files = if cli.accumulate {
        cascade::expand(&cli.files[0])?
    } else {
        cli.files.clone()
    };
    if cli.list_files {
        let mut text = String::new();
        for path in &files {
            use std::fmt::Write;
            writeln!(text, "{}", path.display()).expect("writing to a String cannot fail");
        }
        return write_stdout(&text);
    }

    run_pipeline(&cli, &files, overlays)
}

/// One pipeline for explicit and discovered files: every layer becomes a
/// `Value`, the fold runs once, and the output format is only consulted at emit.
/// Nothing about JSON or TOML reaches the merge.
fn run_pipeline(cli: &Cli, files: &[PathBuf], overlays: Vec<Value>) -> anyhow::Result<()> {
    // Between the parse and the fold: the output format is a decision about
    // argv, and the formats it needs are known as soon as the inputs are read.
    // Deciding it after the merge would make a forgotten `-f` queue behind
    // every error in the documents themselves.
    let (layers, input_formats) =
        load_layers(files, cli.input_format.map(Format::from)).map_err(explain_pipeline)?;
    let out_format = resolve_output_format(cli.format.map(Format::from), &input_formats)?;

    // One flat, strictly-left fold: --set layers are appended after every file.
    let opts = MergeOptions {
        strict: cli.strict,
        shallow: cli.shallow,
    };
    let layers = layers.into_iter().chain(overlays);
    let merged = merge(layers, &opts).map_err(explain_pipeline)?;
    // After the merge, before the emit, and never per layer.
    let merged = if cli.interpolate {
        interpolate(merged, &ProcessEnv).map_err(explain_pipeline)?
    } else {
        merged
    };
    let text = format::emit(merged, out_format, !cli.compact, cli.null_as.as_deref())
        .map_err(explain_pipeline)?;
    write_stdout(&text)
}

/// Builds the `--set` layers, validating every expression up front.
///
/// Fallible, and called before any input is read: the expressions come from
/// argv alone, so nothing about the files can change whether they are legal.
fn overlays(cli: &Cli) -> anyhow::Result<Vec<Value>> {
    // --set layers are terminal: appended after every file. The RHS parses as
    // JSON with a string fallback, which is `knf-core`'s job. The conversion
    // is also where a bracketed path is rejected, so it runs up front, not
    // after the files exist or parse.
    let mut overlays: Vec<Value> = Vec::with_capacity(cli.set.len());
    for path_leaf in &cli.set {
        let typed = PathLeaf::<serde_json::Value>::from(path_leaf.clone());
        let json = serde_json::Value::try_from(typed).map_err(name_the_set_flag)?;
        let serde_json::Value::Object(obj) = json else {
            // The expansion nests the leaf under every key in the path, and the
            // grammar rejects an empty path — so it is always an object.
            unreachable!("a --set expression expands to a nested object")
        };
        overlays.push(Value::Object(knf::value::object_from_json(obj)));
    }
    Ok(overlays)
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
