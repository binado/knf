//! clap derive structs.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::format::Format;

#[derive(Parser, Debug)]
#[command(
    name = "knf",
    version,
    about = "Merge layered configuration files and print the result",
    long_about = "\
Merge layered configuration files and print the result.

Files are layers, merged left to right in argument order. JSON and TOML may be
mixed freely. Exactly one document goes to stdout.

  knf base.toml prod.toml
  knf base.json - --input-format json          # stdin as a layer
  knf defaults.json --set server.port=8080 -f toml
  knf matrix config/ --out-dir out/            # enumerate a config tree

Objects merge key by key. Arrays, scalars and null all replace wholesale —
null is an ordinary value that overwrites, not a delete instruction.",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub merge: MergeArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Enumerate a directory tree of config groups
    #[command(long_about = "\
Enumerate a directory tree of config groups.

A directory is a group and the files within it are mutually exclusive
alternatives; each subdirectory is an independent axis that also applies. A
group with one file auto-selects; a group with several must be pinned, or every
combination is produced.

  knf matrix config/ --list                    # show what would be produced
  knf matrix config/ db=postgres server=nginx  # one document to stdout
  knf matrix config/ --out-dir out/            # every combination, one file each

An argument containing `=` is a group pin; anything else is the directory.

Known limitation: a file named exactly `matrix` in the current directory is
parsed as this subcommand. Write `./matrix` to disambiguate.")]
    Matrix(MatrixArgs),
}

/// Flags meaningful to both commands.
///
/// Flattened separately into each because `args_conflicts_with_subcommands`
/// makes top-level arguments unusable alongside a subcommand.
#[derive(Args, Debug)]
pub struct CommonArgs {
    /// Output format; required when inputs are mixed
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<Format>,

    /// Error when a layer changes the type of an existing key
    #[arg(long)]
    pub strict: bool,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,
}

#[derive(Args, Debug)]
pub struct MergeArgs {
    /// Files to merge as layers; `-` reads stdin
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Treat every input as this format; required for `-`
    #[arg(
        long,
        value_name = "FORMAT",
        long_help = "\
Treat every input as this format, overriding extension inference.

Required for `-`, which has no extension. Note that it applies to all inputs,
not only stdin, so it cannot be used to mix a stdin layer of one format with
files of another."
    )]
    pub input_format: Option<Format>,

    /// Inline terminal layer, applied after all files
    #[arg(
        long = "set",
        value_name = "KEY.PATH=VALUE",
        long_help = "\
Inline terminal layer, applied after all files. Repeatable; multiple --set apply
left to right.

The value is parsed as JSON, falling back to a string when that fails:

  port=8080       -> 8080     (number)
  debug=true      -> true     (bool)
  name=foo        -> \"foo\"    (not valid JSON, so a string)
  proxy=null      -> null     (a value, not a delete)
  tags=[\"a\",\"b\"]  -> array
  tags=[a,b]      -> \"[a,b]\"  (not valid JSON, so a string)

Sharp edge: version=1.0 is the number 1.0, not the string \"1.0\". Force a string
by quoting into JSON: --set version='\"1.0\"'.

Dotted paths nest, so keys containing a literal dot are not addressable from
--set; use a file."
    )]
    pub set: Vec<String>,

    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Args, Debug)]
pub struct MatrixArgs {
    /// The directory, plus any GROUP=CHOICE pins, in any order
    #[arg(value_name = "DIR|GROUP=CHOICE")]
    pub args: Vec<String>,

    /// Write one file per combination here instead of to stdout
    #[arg(
        long,
        value_name = "DIR",
        long_help = "\
Write one file per combination here instead of to stdout.

Filenames are the GROUP=CHOICE pairs joined by --separator, sorted by group so a
combination always gets the same name. Groups with a single alternative are
omitted; pinned groups are kept, which is what makes the name reversible.

A nested group keeps the slash in its id, so `db/tuning=small` puts the file one
directory down even without --tree. Use --tree for nested trees."
    )]
    pub out_dir: Option<PathBuf>,

    /// Nest output directories instead of using flat filenames
    #[arg(long)]
    pub tree: bool,

    /// Joiner between GROUP=CHOICE pairs in filenames
    #[arg(long, value_name = "SEP", default_value = ",")]
    pub separator: String,

    /// Print the resolved combinations and write nothing
    #[arg(long)]
    pub list: bool,

    /// Refuse to produce more than this many documents
    #[arg(long, value_name = "N", default_value_t = 256)]
    pub max: usize,

    #[command(flatten)]
    pub common: CommonArgs,
}
