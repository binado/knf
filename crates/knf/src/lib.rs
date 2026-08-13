//! The `knf` pipeline: load layers, merge, emit.

pub mod cli;
pub mod format;
pub mod value;

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use knf_merge::{MergeOptions, merge_all_json, merge_all_toml, merge_json, merge_toml};
use serde_json::Value;

use cli::Cli;
use format::{Format, SourceName};
use value::{Document, Json, Toml};

/// The positional that means "read stdin".
const STDIN: &str = "-";

/// `knf <files...>` — merge layers left to right, print one document.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    let mut files: Vec<(SourceName, Document)> = Vec::new();
    let mut input_formats: Vec<Format> = Vec::new();

    for path in &cli.files {
        let (name, format, text) = read_input(path, cli.input_format)?;
        let doc = format::parse(format, &text, &name)?;
        input_formats.push(format);
        files.push((name, doc));
    }

    // --set layers are terminal: appended after every file. Folded among
    // themselves first so `--set a=null --set a=1` can convert to TOML as one
    // document (the null does not survive).
    let mut set_layers: Vec<(SourceName, Json)> = Vec::new();
    for path_leaf in &cli.set {
        set_layers.push((
            SourceName::Set(path_leaf.to_string()),
            Json(Value::from(path_leaf.clone())),
        ));
    }

    let out_format = resolve_output_format(cli.format, &input_formats)?;
    let opts = merge_options(&cli);
    let merged = merge_layers(files, set_layers, out_format, &opts)?;
    let text = format::emit(merged, !cli.compact)?;
    write_stdout(&text)
}

/// Merge in the native type of `out` whenever every file already has that type.
/// Otherwise convert foreign layers into JSON, merge, and convert once at the end.
fn merge_layers(
    files: Vec<(SourceName, Document)>,
    set_layers: Vec<(SourceName, Json)>,
    out: Format,
    opts: &MergeOptions,
) -> anyhow::Result<Document> {
    let all_files_toml = !files.is_empty()
        && files
            .iter()
            .all(|(_, doc)| matches!(doc, Document::Toml(_)));

    if all_files_toml {
        return merge_native_toml(files, set_layers, out, opts);
    }

    merge_via_json(files, set_layers, out, opts)
}

fn merge_native_toml(
    files: Vec<(SourceName, Document)>,
    set_layers: Vec<(SourceName, Json)>,
    out: Format,
    opts: &MergeOptions,
) -> anyhow::Result<Document> {
    let file_layers = files.into_iter().map(|(_, doc)| match doc {
        Document::Toml(Toml(v)) => v,
        Document::Json(_) => unreachable!("filtered to TOML files"),
    });
    let acc = merge_all_toml(file_layers, opts)?;
    match out {
        Format::Toml => {
            let mut acc = acc;
            if !set_layers.is_empty() {
                let folded = merge_all_json(set_layers.iter().map(|(_, Json(v))| v.clone()), opts)?;
                let Toml(set_toml) =
                    Toml::try_from(Json(folded)).map_err(|e| e.with_origins(&set_layers))?;
                merge_toml(&mut acc, set_toml, opts)?;
            }
            Ok(Document::Toml(Toml(acc)))
        }
        Format::Json => {
            let mut json = Json::from(Toml(acc)).0;
            for (_, Json(set)) in set_layers {
                merge_json(&mut json, set, opts)?;
            }
            Ok(Document::Json(Json(json)))
        }
    }
}

fn merge_via_json(
    files: Vec<(SourceName, Document)>,
    set_layers: Vec<(SourceName, Json)>,
    out: Format,
    opts: &MergeOptions,
) -> anyhow::Result<Document> {
    let mut json_layers = Vec::new();
    let mut json_sources: Vec<(SourceName, Json)> = Vec::new();
    for (name, doc) in files {
        match doc {
            Document::Json(j) => {
                json_sources.push((name, j.clone()));
                json_layers.push(j.0);
            }
            Document::Toml(t) => json_layers.push(Json::from(t).0),
        }
    }
    for (name, j) in set_layers {
        json_sources.push((name, j.clone()));
        json_layers.push(j.0);
    }

    let merged = merge_all_json(json_layers, opts)?;
    match out {
        Format::Json => Ok(Document::Json(Json(merged))),
        Format::Toml => {
            let toml = Toml::try_from(Json(merged)).map_err(|e| e.with_origins(&json_sources))?;
            Ok(Document::Toml(toml))
        }
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
