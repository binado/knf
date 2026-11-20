//! `knf <files...>`: merge layers left to right and print one document.

mod cli;
mod explain;

use std::ffi::OsString;
use std::io::Write;
use std::path::{PathBuf, absolute};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};
use knf::{
    ConfigFormat, InterpOptions, Layers, MergeOptions, ProcessEnv, format, load_layers, merge,
    merge_interpolate,
};

use cli::{Cli, ExecFormat, OutputFormat};
use explain::{explain_pipeline, name_the_inline_layer_flag};
#[cfg(feature = "yaml")]
use knf_convert::emit_yaml;
use knf_convert::{Converted, Policy};

/// Set while an `--exec` child runs, so Ctrl-C is left to the child.
static CHILD_RUNNING: AtomicBool = AtomicBool::new(false);

// Unused when included by `knf-py`, which calls `main_from` directly and keeps
// Python's SIGINT handler.
#[allow(dead_code)]
fn main() {
    // Terminal Ctrl-C hits parent and child together; stay alive while a child
    // runs so the snapshot tempfile is removed on drop.
    if let Err(err) = ctrlc::set_handler(|| {
        if !CHILD_RUNNING.load(Ordering::SeqCst) {
            std::process::exit(130);
        }
    }) {
        eprintln!("warning: Ctrl-C will not clean up the --exec snapshot: {err}");
    }
    main_from(std::env::args_os());
}

/// Runs the command line over `args`, whose first item is the program name.
pub fn main_from<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let (cli, command) = Cli::parse_command(args.into_iter().map(Into::into));

    match run(cli, &command) {
        Ok(0) => (),
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("error: {err}");
            for cause in err.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::exit(1);
        }
    }
}

fn run(cli: Cli, command: &[OsString]) -> anyhow::Result<i32> {
    // Validate -c paths before any file I/O.
    for leaf in &cli.set {
        leaf.validate_keys().map_err(name_the_inline_layer_flag)?;
    }
    let opts = MergeOptions {
        shallow: cli.shallow.clone(),
    };

    let mut files = cli.files.clone();
    if let Some(pattern) = &cli.glob {
        files = knf::fs::filter_paths(&files, pattern, false);
    } else if let Some(pattern) = &cli.glob_filename {
        files = knf::fs::filter_paths(&files, pattern, true);
    }
    if cli.list_files {
        let mut text = String::new();
        for path in &files {
            use std::fmt::Write;
            writeln!(text, "{}", path.display()).expect("writing to a String cannot fail");
        }
        write_stdout(&text)?;
        return Ok(0);
    }

    let (text, format) = run_pipeline(&cli, &files, &opts)?;
    if let Some((program, args)) = command.split_first() {
        let format = ExecFormat::try_from(format).map_err(anyhow::Error::msg)?;
        run_child(program, args, &text, format)
    } else {
        write_stdout(&text)?;
        Ok(0)
    }
}

fn run_pipeline(
    cli: &Cli,
    files: &[PathBuf],
    opts: &MergeOptions,
) -> anyhow::Result<(String, OutputFormat)> {
    let mut inputs = files.to_vec();
    inputs.extend(cli.with.iter().cloned());
    let layers = if inputs.is_empty() {
        match cli.to {
            Some(OutputFormat::Json) => Layers::Json(Vec::new()),
            #[cfg(feature = "yaml")]
            Some(OutputFormat::Yaml) => Layers::Json(Vec::new()),
            Some(OutputFormat::Toml) => Layers::Toml(Vec::new()),
            None => {
                bail!("no input files to infer a format from\nhelp: pass --to json or --to toml")
            }
        }
    } else {
        load_layers(&inputs).map_err(|err| explain_pipeline(err, cli.with.as_deref()))?
    };
    let pretty = !cli.compact;
    let policy = if cli.strict {
        Policy::Strict
    } else {
        Policy::Lossy
    };
    let (format, text) = match layers {
        Layers::Json(layers) => {
            let merged = resolve_native(cli, layers, opts)?;
            let output = cli.to.unwrap_or(OutputFormat::Json);
            let text = match output {
                OutputFormat::Json => format::emit(merged, pretty),
                OutputFormat::Toml => format::emit(
                    converted_value(knf_convert::json_to_toml(merged, policy)?),
                    pretty,
                ),
                #[cfg(feature = "yaml")]
                OutputFormat::Yaml => emit_yaml(&merged),
            };
            (output, text)
        }
        Layers::Toml(layers) => {
            let merged = resolve_native(cli, layers, opts)?;
            let output = cli.to.unwrap_or(OutputFormat::Toml);
            let text = match output {
                OutputFormat::Toml => format::emit(merged, pretty),
                OutputFormat::Json => format::emit(
                    converted_value(knf_convert::toml_to_json(merged, policy)?),
                    pretty,
                ),
                #[cfg(feature = "yaml")]
                OutputFormat::Yaml => emit_yaml(&converted_value(knf_convert::toml_to_yaml_ready(
                    merged, policy,
                )?)),
            };
            (output, text)
        }
    };
    Ok((text.map_err(|err| explain_pipeline(err, None))?, format))
}

fn converted_value<V>(converted: Converted<V>) -> V {
    if converted.lossy {
        eprintln!("warning: conversion was lossy; rerun with --strict to find the affected keys");
    }
    converted.value
}

fn resolve_native<V: ConfigFormat>(
    cli: &Cli,
    mut layers: Vec<V>,
    opts: &MergeOptions,
) -> anyhow::Result<V> {
    let context = cli
        .with
        .as_ref()
        .map(|_| layers.pop().expect("context was loaded last"));
    for leaf in &cli.set {
        layers.push(
            leaf.clone()
                .into_layer()
                .map_err(name_the_inline_layer_flag)?,
        );
    }
    // References merge as the values they name, so they fold with the layers.
    if cli.interpolate {
        let options = InterpOptions {
            shallow: cli.shallow.clone(),
        };
        merge_interpolate(layers, context.as_ref(), &ProcessEnv, &options)
            .map_err(|err| explain::explain_interp(err, context.is_some()))
    } else {
        Ok(merge(layers, opts))
    }
}

fn run_child(
    program: &OsString,
    args: &[OsString],
    text: &str,
    format: ExecFormat,
) -> anyhow::Result<i32> {
    let mut snapshot = tempfile::Builder::new()
        .prefix("knf-")
        .suffix(&format!(".{}", format.extension()))
        .tempfile()
        .context("creating configuration snapshot")?;
    snapshot
        .write_all(text.as_bytes())
        .context("writing configuration snapshot")?;
    let path = absolute(snapshot.path()).context("resolving configuration snapshot path")?;
    CHILD_RUNNING.store(true, Ordering::SeqCst);
    let status = Command::new(program)
        .args(args)
        .env("KNF_CONFIG_PATH", path)
        .status()
        .with_context(|| format!("running {program:?}"));
    CHILD_RUNNING.store(false, Ordering::SeqCst);
    let status = status?;
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return Ok(128 + signal);
    }
    Ok(status.code().unwrap_or(1))
}

/// Writes to stdout, ignoring `BrokenPipe` (e.g. `knf big.json | head`).
fn write_stdout(text: &str) -> anyhow::Result<()> {
    match std::io::stdout().write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e).context("writing to stdout"),
    }
}
