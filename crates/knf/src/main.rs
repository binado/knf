//! `knf <files...>`: merge configs left to right and print one document.

mod cli;
mod explain;

use std::ffi::OsString;
use std::io::Write;
use std::path::{PathBuf, absolute};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use knf::{InterpolationOptions, MergeOptions, merge_configs};
use knf_convert::{Configs, Emit, InputFormat, Layout, LoadOptions, Policy, load_configs};

use cli::{Cli, Exec, Invocation, OutputFormat};
use explain::explain_load;

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
    let invocation = Cli::parse_command(args.into_iter().map(Into::into));

    match run(invocation) {
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

fn run(invocation: Invocation) -> anyhow::Result<i32> {
    let Invocation {
        cli,
        filter,
        to,
        exec,
    } = invocation;
    let opts = MergeOptions {
        shallow: cli.shallow.clone(),
    };

    let mut files = cli.files.clone();
    if let Some((pattern, target)) = &filter {
        files = knf::fs::filter_paths(&files, pattern, *target);
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

    let (text, inputs) = run_pipeline(&cli, to, &files, &opts)?;
    if let Some(exec) = exec {
        let format = exec.format.unwrap_or(inputs);
        run_child(&exec, &text, format)
    } else {
        write_stdout(&text)?;
        Ok(0)
    }
}

/// The emitted document, and the format of the inputs.
fn run_pipeline(
    cli: &Cli,
    to: Option<Emit>,
    files: &[PathBuf],
    opts: &MergeOptions,
) -> anyhow::Result<(String, InputFormat)> {
    let Configs {
        format,
        mut values,
        context,
    } = load_configs(
        files,
        LoadOptions {
            context: cli.with.as_deref(),
            fallback: cli.to.map(OutputFormat::inline_format),
        },
    )
    .map_err(explain_load)?;
    let parse_inline = |text| format.parse_inline(text);
    for leaf in &cli.set {
        values.push(leaf.clone().map_leaf(parse_inline).into());
    }
    let interpolation = cli
        .interpolate
        .then(|| InterpolationOptions::new(&parse_inline).with_context(context.as_ref()));
    let merged = merge_configs(values, opts, interpolation)
        .map_err(|err| explain::explain_interp(err, cli.with.as_deref()))?;
    let emit = to.unwrap_or_else(|| Emit::matching(format, Layout::pretty(!cli.compact)));
    let converted = emit.emit(&merged, Policy::strict(cli.strict))?;
    if converted.lossy {
        eprintln!("warning: conversion was lossy; rerun with --strict to find the affected keys");
    }
    Ok((converted.value, format))
}

fn run_child(exec: &Exec, text: &str, format: InputFormat) -> anyhow::Result<i32> {
    let Exec { program, args, .. } = exec;
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
