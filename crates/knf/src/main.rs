//! `knf <files...>`: merge layers left to right and print one document.

mod accumulate;
mod cli;
mod explain;

use std::ffi::OsString;
use std::io::Write;
use std::path::{PathBuf, absolute};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use knf::{
    ConfigFormat, Format, InterpOptions, Layers, MergeOptions, ProcessEnv, format, load_layers,
    merge, merge_interpolate,
};

use cli::Cli;
use explain::{explain_pipeline, name_the_inline_layer_flag};
use knf_convert::{Converted, Policy};

/// Set while an `--exec` child runs, so Ctrl-C is left to the child.
static CHILD_RUNNING: AtomicBool = AtomicBool::new(false);

// Entry point for the `knf-cli` binary. `knf-py` includes this file and calls
// `main_from` instead, so the function is unused in that compilation.
#[allow(dead_code)]
fn main() {
    // The pyknf wheel calls `main_from` with `sys.argv` instead, and keeps
    // Python's own SIGINT handling.
    //
    // A terminal Ctrl-C reaches `knf` and its child together. While a child
    // runs, `knf` outlives it so the snapshot is removed on drop; otherwise it
    // exits as a default SIGINT would.
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
    // clap handles --help/--version and exits 2 on usage errors.
    let (cli, command) = Cli::parse_command(args.into_iter().map(Into::into));

    match run(cli, &command) {
        Ok(0) => (),
        Ok(code) => std::process::exit(code),
        Err(err) => {
            // Frontend help is added by `explain`; library errors remain flag-free.
            eprintln!("error: {err}");
            for cause in err.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::exit(1);
        }
    }
}

/// Prepare explicit file inputs and terminal overlays for the shared pipeline.
fn run(cli: Cli, command: &[OsString]) -> anyhow::Result<i32> {
    // Validate -c paths before any file I/O.
    for leaf in &cli.set {
        leaf.validate_keys().map_err(name_the_inline_layer_flag)?;
    }
    let opts = MergeOptions {
        shallow: cli.shallow.clone(),
    };

    let mut files = if let Some(target) = &cli.accumulate {
        knf::fs::accumulate(target, None).map_err(explain::explain_accumulate)?
    } else {
        cli.files.clone()
    };
    if let Some(pattern) = &cli.glob {
        files = knf::fs::filter_paths(&files, pattern, false);
    } else if let Some(pattern) = &cli.glob_filename {
        files = knf::fs::filter_paths(&files, pattern, true);
    }
    cli.validate_stdin(&files);
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
        run_child(program, args, &text, format)
    } else {
        write_stdout(&text)?;
        Ok(0)
    }
}

/// Resolve natively and convert only the final document when requested.
fn run_pipeline(
    cli: &Cli,
    files: &[PathBuf],
    opts: &MergeOptions,
) -> anyhow::Result<(String, Format)> {
    let mut inputs = files.to_vec();
    inputs.extend(cli.with.iter().cloned());
    let layers = load_layers(&inputs, cli.format.map(Into::into))
        .map_err(|err| explain_pipeline(err, cli.with.as_deref()))?;
    let pretty = !cli.compact;
    let policy = if cli.strict {
        Policy::Strict
    } else {
        Policy::Lossy
    };
    let to = cli.to.map(Format::from);
    let (format, text) = match layers {
        Layers::Json(layers) => {
            let merged = resolve_native(cli, layers, opts)?;
            if to == Some(Format::Toml) {
                let converted = knf_convert::json_to_toml(merged, policy)?;
                (Format::Toml, emit_converted(converted, pretty))
            } else {
                (Format::Json, format::emit(merged, pretty))
            }
        }
        Layers::Toml(layers) => {
            let merged = resolve_native(cli, layers, opts)?;
            if to == Some(Format::Json) {
                let converted = knf_convert::toml_to_json(merged, policy)?;
                (Format::Json, emit_converted(converted, pretty))
            } else {
                (Format::Toml, format::emit(merged, pretty))
            }
        }
    };
    Ok((text.map_err(|err| explain_pipeline(err, None))?, format))
}

/// Warns when the conversion was lossy, then serializes the converted value.
fn emit_converted<V: ConfigFormat>(
    converted: Converted<V>,
    pretty: bool,
) -> anyhow::Result<String> {
    if converted.lossy {
        eprintln!("warning: conversion was lossy; rerun with --strict to find the affected keys");
    }
    format::emit(converted.value, pretty)
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
            merge_key: cli.merge_key.clone(),
            shallow: cli.shallow.clone(),
        };
        merge_interpolate(layers, context.as_ref(), &ProcessEnv, &options)
            .map_err(|err| explain::explain_interp(err, context.is_some(), cli.merge_key.is_some()))
    } else {
        Ok(merge(layers, opts))
    }
}

fn run_child(
    program: &OsString,
    args: &[OsString],
    text: &str,
    format: Format,
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

/// Writes to stdout, treating a closed pipe as success so `knf big.json | head`
/// does not report an error the user cannot act on.
fn write_stdout(text: &str) -> anyhow::Result<()> {
    match std::io::stdout().write_all(text.as_bytes()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e).context("writing to stdout"),
    }
}
