//! `knf <files...>`: merge layers left to right and print one document.

mod cli;
mod explain;

use std::ffi::OsString;
use std::io::Write;
use std::path::{PathBuf, absolute};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};
use knf::{InterpolationOptions, MergeOptions, ProcessEnv, merge, merge_with_interpolation};
use knf_convert::{InputFormat, Layers, Policy, load_layers};

use cli::{Cli, ExecFormat, OutputFormat};
use explain::{explain_pipeline, name_the_inline_layer_flag};

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
    let Layers { format, mut values } = if inputs.is_empty() {
        let format = match cli.to {
            Some(OutputFormat::Toml) => InputFormat::Toml,
            Some(_) => InputFormat::Json,
            None => {
                bail!("no input files to infer a format from\nhelp: pass --to json or --to toml")
            }
        };
        Layers {
            format,
            values: Vec::new(),
        }
    } else {
        load_layers(&inputs).map_err(|err| explain_pipeline(err, cli.with.as_deref()))?
    };
    let context = cli
        .with
        .as_ref()
        .map(|_| values.pop().expect("context was loaded last"));
    let parse_inline = |text| format.parse_inline(text);
    for leaf in &cli.set {
        values.push(
            leaf.clone()
                .map_leaf(parse_inline)
                .try_into()
                .map_err(name_the_inline_layer_flag)?,
        );
    }
    let merged = if cli.interpolate {
        merge_with_interpolation(
            values,
            opts,
            &InterpolationOptions {
                context: context.as_ref(),
                env: &ProcessEnv,
                parse_inline: &parse_inline,
            },
        )
        .map_err(|err| explain::explain_interp(err, context.is_some()))?
    } else {
        merge(values, opts)
    };
    let output = cli.to.unwrap_or(match format {
        InputFormat::Json => OutputFormat::Json,
        InputFormat::Toml => OutputFormat::Toml,
    });
    let target = match output {
        OutputFormat::Json => knf_convert::OutputFormat::Json,
        OutputFormat::Toml => knf_convert::OutputFormat::Toml,
        #[cfg(feature = "yaml")]
        OutputFormat::Yaml => knf_convert::OutputFormat::Yaml,
    };
    let policy = if cli.strict {
        Policy::Strict
    } else {
        Policy::Lossy
    };
    let converted = target.emit(&merged, policy, !cli.compact)?;
    if converted.lossy {
        eprintln!("warning: conversion was lossy; rerun with --strict to find the affected keys");
    }
    Ok((converted.value, output))
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
