//! `knf <files...>`: merge layers left to right and print one document.

mod accumulate;
mod cli;
mod explain;

use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use knf::{
    ConfigFormat, Layers, MergeOptions, ProcessEnv, format, interpolate, interpolate_with_context,
    load_layers, merge,
};

use cli::Cli;
use explain::{explain_pipeline, name_the_inline_layer_flag};

// Entry point for the `knf-cli` binary. `knf-py` includes this file and calls
// `main_from` instead, so the function is unused in that compilation.
#[allow(dead_code)]
fn main() {
    // The pyknf wheel calls `main_from` with `sys.argv` instead.
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

    if let Err(err) = run(cli) {
        // Frontend help is added by `explain`; library errors remain flag-free.
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        std::process::exit(1);
    }
}

/// Prepare explicit file inputs and terminal overlays for the shared pipeline.
fn run(cli: Cli) -> anyhow::Result<()> {
    // Validate -c paths before any file I/O.
    for leaf in &cli.set {
        leaf.validate_keys().map_err(name_the_inline_layer_flag)?;
    }
    let opts = MergeOptions {
        strict: cli.strict,
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
        return write_stdout(&text);
    }

    run_pipeline(&cli, &files, &opts)
}

/// Dispatch once to a native pipeline; every later stage retains its type.
fn run_pipeline(cli: &Cli, files: &[PathBuf], opts: &MergeOptions) -> anyhow::Result<()> {
    let mut inputs = files.to_vec();
    inputs.extend(cli.with.iter().cloned());
    let layers = load_layers(&inputs, cli.format.map(Into::into)).map_err(explain_pipeline)?;
    match layers {
        Layers::Json(layers) => run_native(cli, layers, opts),
        Layers::Toml(layers) => run_native(cli, layers, opts),
    }
}

fn run_native<V: ConfigFormat>(
    cli: &Cli,
    mut layers: Vec<V>,
    opts: &MergeOptions,
) -> anyhow::Result<()> {
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
    let merged = merge(layers, opts).map_err(explain_pipeline)?;
    let merged = if cli.interpolate {
        let resolved = match &context {
            Some(context) => interpolate_with_context(merged, context, &ProcessEnv),
            None => interpolate(merged, &ProcessEnv),
        };
        resolved.map_err(|err| explain::explain_interp(err, context.is_some()))?
    } else {
        merged
    };
    let text = format::emit(merged, !cli.compact).map_err(explain_pipeline)?;
    write_stdout(&text)
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
