//! The `knf` pipeline: load layers, merge, emit.

pub mod cli;
pub mod format;
pub mod set;

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use knf_merge::{MergeOptions, merge_all};

use cli::Cli;
use format::{Format, Source, SourceName};

/// The positional that means "read stdin".
const STDIN: &str = "-";

/// `knf <files...>` — merge layers left to right, print one document.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    let mut sources: Vec<Source> = Vec::new();
    let mut input_formats: Vec<Format> = Vec::new();

    for path in &cli.files {
        let (name, format, text) = read_input(path, cli.input_format)?;
        let value = format::parse(format, &text, &name)?;
        input_formats.push(format);
        sources.push((name, value));
    }

    // --set layers are terminal: appended after every file, folded through the
    // same merge, so --strict applies to them too.
    for expr in &cli.set {
        sources.push((SourceName::Set(expr.clone()), set::parse(expr)?));
    }

    let out_format = resolve_output_format(cli.format, &input_formats)?;
    let merged = merge_all(sources.iter().map(|(_, v)| v.clone()), &merge_options(&cli))?;
    let text = format::emit(out_format, merged, !cli.compact, &sources)?;
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

pub fn merge_options(cli: &Cli) -> MergeOptions {
    MergeOptions { strict: cli.strict }
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
