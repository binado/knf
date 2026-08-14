//! The `knf` pipeline: load layers, merge, emit.

pub mod cli;
pub mod format;
mod product;
pub mod value;

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use knf_core::{MergeOptions, Value, merge_with};
use knf_dotted::PathLeaf;
use tempfile::NamedTempFile;

use cli::Cli;
use format::{Format, SourceName};
use product::{
    Combination, LoadedLayer, enumerate_combinations, file_label, is_product, load_set,
    split_factors,
};

/// The positional that means "read stdin".
pub(crate) const STDIN: &str = "-";

/// Runs ordinary single-document mode or Cartesian-product generation.
///
/// One pipeline regardless of the formats involved: every selected layer
/// becomes a [`Value`], each output performs one flat left fold, and the output
/// format is only consulted at emit. Nothing about JSON or TOML reaches the
/// merge.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.files.iter().any(|path| is_product(path)) {
        run_product(cli)
    } else {
        run_single(cli)
    }
}

fn run_single(cli: Cli) -> anyhow::Result<()> {
    if cli.output_dir.is_some() {
        bail!("--output-dir requires Cartesian-product mode with a positional `x`");
    }
    if cli.output_separator.is_some() {
        bail!("--output-separator requires Cartesian-product mode with a positional `x`");
    }

    let mut layers: Vec<(SourceName, Value)> = Vec::new();
    let mut input_formats: Vec<Format> = Vec::new();

    for path in &cli.files {
        let (name, format, text) = read_input(path, cli.input_format)?;
        let value = format::parse(format, &text, &name)?;
        input_formats.push(format);
        layers.push((name, value));
    }

    // --set layers are terminal: appended after every file. The RHS parses as
    // JSON with a string fallback, which is knf-dotted's job.
    for path_leaf in &cli.set {
        let name = SourceName::Set(path_leaf.to_string());
        let json = serde_json::Value::from(PathLeaf::<serde_json::Value>::from(path_leaf.clone()));
        layers.push((name, value::from_json(json)));
    }

    let out_format = resolve_output_format(cli.format, &input_formats)?;
    let opts = merge_options(&cli);

    // Provenance is only ever read by the null-in-TOML error, so JSON output
    // does not pay for the clone.
    let sources: Vec<(SourceName, Value)> = match out_format {
        Format::Toml => layers.clone(),
        Format::Json => Vec::new(),
    };

    let merged = merge_with(layers.into_iter().map(|(_, value)| value), &opts)?;
    let text = format::emit(merged, out_format, !cli.compact, &sources)?;
    write_stdout(&text)
}

fn run_product(cli: Cli) -> anyhow::Result<()> {
    let path_factors = split_factors(&cli.files)?;
    let output_dir = cli
        .output_dir
        .as_ref()
        .context("Cartesian-product mode requires --output-dir <DIR>")?;

    let mut input_formats = Vec::new();
    let mut factors = Vec::with_capacity(path_factors.len() + usize::from(!cli.set.is_empty()));
    for factor in path_factors {
        let mut loaded = Vec::with_capacity(factor.len());
        for path in factor {
            let (source, input_format, text) = read_input(path, cli.input_format)?;
            let parsed = format::parse(input_format, &text, &source)?;
            input_formats.push(input_format);
            loaded.push(LoadedLayer {
                source,
                value: parsed,
                label: file_label(path)?,
            });
        }
        factors.push(loaded);
    }

    // Unlike ordinary mode, repeated --set expressions are alternatives in one
    // final factor. One --set still appears in every generated configuration.
    if !cli.set.is_empty() {
        factors.push(
            cli.set
                .iter()
                .map(load_set)
                .collect::<anyhow::Result<Vec<_>>>()?,
        );
    }

    let out_format = resolve_output_format(cli.format, &input_formats)?;
    let separator = cli.output_separator.as_deref().unwrap_or("+");
    if separator.is_empty() {
        bail!("output separator cannot be empty");
    }
    if separator.contains(['/', '\\', '\0']) {
        bail!("output separator cannot contain path separators or null bytes: `{separator}`");
    }

    let combinations = enumerate_combinations(&factors, separator, out_format)?;
    validate_output_dir(output_dir)?;
    preflight_outputs(&combinations, output_dir)?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("creating output directory `{}`", output_dir.display()))?;

    let opts = merge_options(&cli);
    let mut prepared = Vec::new();
    for combination in &combinations {
        let selected = &combination.selected;
        let destination = output_dir.join(&combination.name);

        let sources: Vec<(SourceName, Value)> = match out_format {
            Format::Toml => selected
                .iter()
                .map(|layer| (layer.source.clone(), layer.value.clone()))
                .collect(),
            Format::Json => Vec::new(),
        };
        let merged = merge_with(selected.iter().map(|layer| layer.value.clone()), &opts)
            .with_context(|| format!("generating `{}`", destination.display()))?;
        let text = format::emit(merged, out_format, !cli.compact, &sources)
            .with_context(|| format!("generating `{}`", destination.display()))?;

        let mut temporary = NamedTempFile::new_in(output_dir)
            .with_context(|| format!("creating a temporary file in `{}`", output_dir.display()))?;
        temporary
            .write_all(text.as_bytes())
            .with_context(|| format!("writing temporary output for `{}`", destination.display()))?;
        temporary.flush().with_context(|| {
            format!("flushing temporary output for `{}`", destination.display())
        })?;
        prepared.push((temporary.into_temp_path(), destination));
    }

    let mut written = Vec::with_capacity(prepared.len());
    for (temporary, destination) in prepared {
        match temporary.persist_noclobber(&destination) {
            Ok(_) => written.push(destination),
            Err(error) => {
                for path in &written {
                    let _ = fs::remove_file(path);
                }
                return Err(error.error)
                    .with_context(|| format!("creating `{}`", destination.display()));
            }
        }
    }

    let mut listing = String::new();
    for path in &written {
        listing.push_str(&path.display().to_string());
        listing.push('\n');
    }
    write_stdout(&listing)
}

fn validate_output_dir(output_dir: &Path) -> anyhow::Result<()> {
    if output_dir.exists() && !output_dir.is_dir() {
        bail!(
            "output directory `{}` exists but is not a directory",
            output_dir.display()
        );
    }
    Ok(())
}

fn preflight_outputs(combinations: &[Combination], output_dir: &Path) -> anyhow::Result<()> {
    let mut names = HashSet::new();
    for combination in combinations {
        if !names.insert(combination.name.as_str()) {
            bail!(
                "multiple combinations produce the output name `{}`",
                combination.name
            );
        }
        let destination = output_dir.join(&combination.name);
        if destination
            .try_exists()
            .with_context(|| format!("checking `{}`", destination.display()))?
        {
            bail!(
                "refusing to overwrite existing output `{}`",
                destination.display()
            );
        }
    }
    Ok(())
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
