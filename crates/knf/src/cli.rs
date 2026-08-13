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
    /// Enumerate saturating paths through a config directory tree
    #[command(long_about = "\
Enumerate saturating paths through a config directory tree.

Files in a directory are mutually exclusive layers for that node.
Subdirectories are alternative branches: a path picks one child and continues,
and files on the way down always apply. One matching path goes to stdout;
several need --out-dir or --list.

  knf matrix config/ --list                    # show what would be produced
  knf matrix config/ --glob 'db/postgres.toml' # one document to stdout
  knf matrix config/ --out-dir out/            # every path, one file each

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
    /// The config directory to walk
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    /// Keep only leaves whose path relative to DIR matches this glob.
    /// Repeatable; multiple --glob form a union. Ancestor files still apply.
    #[arg(long, value_name = "PATTERN")]
    pub glob: Vec<String>,

    /// Do not walk deeper than this. Root is depth 0.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// Write one file per path here instead of to stdout
    #[arg(
        long,
        value_name = "DIR",
        long_help = "\
Write one file per path here instead of to stdout.

Each file is named after the leaf's path relative to the walked directory,
with the output-format extension. Nested leaves keep their slashes, so
db/mysql.toml lands at out/db/mysql.toml. Pass --separator to flatten."
    )]
    pub out_dir: Option<PathBuf>,

    /// Replace `/` in output names with this string, flattening nested leaves
    #[arg(long, value_name = "SEP")]
    pub separator: Option<String>,

    /// Print the resolved paths and write nothing
    #[arg(long)]
    pub list: bool,

    /// Refuse to produce more than this many documents
    #[arg(long, value_name = "N", default_value_t = 256)]
    pub max: usize,

    #[command(flatten)]
    pub common: CommonArgs,
}
