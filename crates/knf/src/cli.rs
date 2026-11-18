//! clap derive structs.

use std::{ffi::OsString, path::PathBuf};

use clap::{CommandFactory, Parser, error::ErrorKind};
use knf::glob::KeyGlobPattern;
use knf::{Format, PathLeaf};

/// `-f` values. Mirrors [`Format`] because the orphan rule forbids deriving
/// `ValueEnum` on it here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FormatArg {
    Json,
    Toml,
}

impl From<FormatArg> for Format {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Json => Format::Json,
            FormatArg::Toml => Format::Toml,
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "knf",
    version,
    about = "Merge layered configuration files and print the result",
    long_about = "\
Merge layered JSON or TOML files, left to right, and print the result.

Objects merge recursively; arrays, scalars and null replace wholesale.

  knf base.toml prod.toml
  knf base.json - -f json
  knf defaults.toml -c server.port=8080
  knf base.toml -c seed=42 --exec -- python fit.py --replicate 2"
)]
pub struct Cli {
    /// Files to merge as layers; `-` reads stdin
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Filter inputs by a case-sensitive glob matching the whole path
    #[arg(
        short = 'g',
        long,
        value_name = "PATTERN",
        conflicts_with = "glob_filename",
        long_help = "\
Keep only inputs whose whole path matches a case-sensitive glob. Supports *,
**, ?, [...], {a,b} and leading ! negation. Stdin matches as -.

  knf config/**/*.toml -g 'config/**/prod.toml'"
    )]
    pub glob: Option<knf::glob::GlobPattern>,

    /// Filter inputs by a case-sensitive glob matching just the filename
    #[arg(
        short = 'G',
        long,
        value_name = "PATTERN",
        conflicts_with = "glob",
        long_help = "\
Like --glob, but match only the filename.

  knf -a services/api/prod.toml -G '{defaults,prod}.toml'"
    )]
    pub glob_filename: Option<knf::glob::GlobPattern>,

    /// Accumulate same-format layers along one relative target path
    #[arg(
        short = 'a',
        long = "accumulate",
        value_name = "TARGET",
        value_parser = super::accumulate::parser(),
        conflicts_with = "files",
        long_help = "\
Collect same-format files from each directory along a relative target path,
shallowest first, sorted by filename, with the target last. Files directly in
the working directory are excluded.

  knf -a foo/bar/config.toml --list-files"
    )]
    pub accumulate: Option<knf::fs::AccumulateTarget>,

    /// Print the file list in merge order without reading contents
    #[arg(short = 'l', long)]
    pub list_files: bool,

    /// Inline terminal layer, applied after all files
    #[arg(
        short = 'c',
        value_name = "KEY.PATH=VALUE",
        long_help = "\
Inline layer applied after all files; repeatable. The value is parsed in the
selected format, falling back to a string.

  -c port=8080  -c name=foo  -c version='\"1.0\"'"
    )]
    pub set: Vec<PathLeaf<String>>,

    /// Replace values whose full key paths match one glob
    #[arg(
        long,
        value_name = "PATTERN",
        long_help = "\
Replace values at matching key paths wholesale instead of merging them. Dots
separate keys; single-quoted spans are literal.

  --shallow '*'        top-level keys (jq's a + b)
  --shallow 'db'       replace db entirely
  --shallow 'db.*'     replace db's children"
    )]
    pub shallow: Option<KeyGlobPattern>,

    /// Format for parsing and inline typing; required for stdin
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Convert the merged document to this output format
    #[arg(
        long,
        value_name = "FORMAT",
        long_help = "\
Convert the document after merging and interpolation. Output defaults to
the input format.

TOML dates, times and datetimes become strings in TOML notation. Non-finite
TOML floats become strings: nan, inf or -inf. JSON null object fields are
omitted, and integers above the TOML signed range become floats. A JSON null
array entry is an error. A lossy conversion prints a warning on stderr; use
--strict to reject it and name the key.

  knf base.toml prod.toml --to json"
    )]
    pub to: Option<FormatArg>,

    /// Reject lossy output conversions
    #[arg(long, requires = "to")]
    pub strict: bool,

    /// Resolve ${key.path} and ${env:VAR} references in the merged document
    #[arg(
        short = 'i',
        long,
        long_help = "\
Resolve ${key.path} and ${env:VAR} references in the merged document.

A whole-string reference keeps the value's type; an embedded one becomes text.
$$ is a literal $. References bind to the final document, and a whole-string
reference merges exactly as the value it names: an object referent merges with
an object from another layer. --shallow forces replacement.

  data_dir = \"${root}/data\"
  port     = \"${env:PORT}\""
    )]
    pub interpolate: bool,

    /// Merge an object reference under this literal key into its parent
    #[arg(
        short = 'm',
        long = "merge-key",
        value_name = "KEY",
        requires = "interpolate",
        long_help = "\
Treat KEY as an inheritance directive; requires --interpolate. Its value must
be one whole-string reference resolving to an object/table. Merge that base
with local fields, which win, and remove the directive. The last layer's
directive wins. Honors --shallow at destination paths.

Inherited fields can be referenced by their destination paths. References
remain absolute. Only surviving inherited values resolve; environment values
remain terminal. KEY is literal, not a path or glob. One base per object.

  [bar]
  extends = \"${foo}\"
  c = 4

  knf config.toml -i -m extends"
    )]
    pub merge_key: Option<String>,

    /// Read an interpolation context without merging it into output
    #[arg(
        long = "with",
        value_name = "FILE",
        requires = "interpolate",
        requires_if("-", "format"),
        long_help = "\
Read one context file for interpolation; requires --interpolate. Each complete
reference path is looked up in the merged document first, then in the context.
Context values and their dependencies resolve only when referenced. Selected
containers keep their own children. The context uses the same format as inputs,
including any -f override. Use - to read context from stdin; requires -f.
Stdin cannot supply both a merge layer and context.

  knf foo.toml bar.toml -i --with config.toml
  generate-config | knf foo.toml -i --with - -f toml"
    )]
    pub with: Option<PathBuf>,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,

    /// Run a command after -- with the merged file in KNF_CONFIG_PATH
    #[arg(
        long,
        conflicts_with_all = ["list_files", "to"],
        long_help = "\
Run the command and arguments after --. KNF_CONFIG_PATH points to a private
temporary JSON or TOML file containing the merged configuration. The file is
removed when the command finishes. The command inherits standard streams and
its exit status is returned.

  knf base.toml -c seed=42 --exec -- python fit.py --replicate 2"
    )]
    pub exec: bool,
}

impl Cli {
    /// Leave clap's file delimiter alone unless `--exec` appears before it.
    pub fn parse_command(args: impl IntoIterator<Item = OsString>) -> (Self, Vec<OsString>) {
        let mut args: Vec<_> = args.into_iter().collect();
        let delimiter = args.iter().skip(1).position(|arg| arg == "--");
        let end = delimiter.map_or(args.len(), |index| index + 1);
        let command =
            if args.iter().take(end).skip(1).any(|arg| arg == "--exec") && end < args.len() {
                let command = args.split_off(end + 1);
                args.pop();
                command
            } else {
                Vec::new()
            };
        let cli = Self::parse_from(args);
        if cli.exec && command.is_empty() {
            Self::command()
                .error(
                    ErrorKind::MissingRequiredArgument,
                    "--exec requires -- followed by a command",
                )
                .exit();
        }
        (cli, command)
    }

    /// Validate stdin sources after filtering, before reading any documents.
    pub fn validate_stdin(&self, files: &[PathBuf]) {
        if self
            .with
            .as_ref()
            .is_some_and(|path| path.as_os_str() == knf::STDIN)
            && files.iter().any(|path| path.as_os_str() == knf::STDIN)
        {
            Self::command()
                .error(
                    ErrorKind::ArgumentConflict,
                    super::explain::context_stdin_conflict(),
                )
                .exit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_accepts_one_required_glob() {
        for args in [
            vec!["--shallow", "{db,app}.*"],
            vec!["--shallow={db,app}.*"],
        ] {
            let cli = Cli::try_parse_from(std::iter::once("knf").chain(args)).unwrap();
            let glob = cli.shallow.unwrap();
            assert!(glob.matches_keys(&["db".into(), "pool".into()]));
            assert!(!glob.matches_keys(&["db".into()]));
        }
        assert!(
            Cli::try_parse_from(["knf", "a.json"])
                .unwrap()
                .shallow
                .is_none()
        );
        for args in [
            vec!["knf", "--shallow"],
            vec!["knf", "--shallow="],
            vec!["knf", "--shallow=a", "--shallow=b"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
