//! clap derive structs.

use std::{ffi::OsString, path::PathBuf};

use clap::{CommandFactory, Parser, error::ErrorKind};
use knf::PathLeaf;
use knf::fs::GlobTarget;
use knf::glob::{GlobPattern, KeyGlobPattern};
use knf_convert::{Emit, InputFormat, Layout};

// `super`, not `crate`: `knf-py` includes `main.rs` as a module.
use super::explain::name_the_inline_config_flag;

/// Formats accepted by `--to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Json,
    Toml,
    #[cfg(feature = "yaml")]
    Yaml,
}

impl From<OutputFormat> for knf_convert::OutputFormat {
    fn from(format: OutputFormat) -> Self {
        match format {
            OutputFormat::Json => Self::Json,
            OutputFormat::Toml => Self::Toml,
            #[cfg(feature = "yaml")]
            OutputFormat::Yaml => Self::Yaml,
        }
    }
}

impl OutputFormat {
    /// How inline values are parsed when there are no files to say. YAML is
    /// output-only, so it takes JSON values.
    pub fn inline_format(self) -> InputFormat {
        match self {
            Self::Json => InputFormat::Json,
            Self::Toml => InputFormat::Toml,
            #[cfg(feature = "yaml")]
            Self::Yaml => InputFormat::Json,
        }
    }

    /// The format of an `--exec` snapshot, if one can be written in it.
    fn snapshot_format(self) -> Option<InputFormat> {
        match self {
            Self::Json => Some(InputFormat::Json),
            Self::Toml => Some(InputFormat::Toml),
            #[cfg(feature = "yaml")]
            Self::Yaml => None,
        }
    }
}

fn to_long_help() -> String {
    let (yaml_note, yaml_floats, example) = if cfg!(feature = "yaml") {
        (
            " YAML is output-only, parses inline -c values as JSON, and cannot be combined\nwith --compact or --exec.",
            "; YAML keeps\nthem as .nan, .inf or -.inf",
            "yaml",
        )
    } else {
        ("", "", "json")
    };
    format!(
        "\
Convert the document after merging and interpolation. Output defaults to
the input format. Required when there are no input or context files.{yaml_note}

TOML dates, times and datetimes become strings in TOML notation. For JSON
output, non-finite TOML floats become strings: nan, inf or -inf{yaml_floats}. For TOML
output, JSON null object fields are omitted, and integers above the TOML signed
range become floats. A JSON null array entry is an error. A lossy conversion
prints a warning on stderr; use --strict to reject it and name the key.

  knf base.toml prod.toml --to {example}"
    )
}

fn parse_inline_config(expr: &str) -> Result<PathLeaf<String>, String> {
    expr.parse().map_err(name_the_inline_config_flag)
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
  knf base.json prod.json --to toml
  knf defaults.toml -c server.port=8080
  knf base.toml -c seed=42 --exec -- python fit.py --replicate 2"
)]
pub struct Cli {
    /// Files to merge, left to right; the extension selects JSON or TOML
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
**, ?, [...], {a,b} and leading ! negation.

  knf config/**/*.toml -g 'config/**/prod.toml'"
    )]
    glob: Option<GlobPattern>,

    /// Filter inputs by a case-sensitive glob matching just the filename
    #[arg(
        short = 'G',
        long,
        value_name = "PATTERN",
        conflicts_with = "glob",
        long_help = "\
Like --glob, but match only the filename.

  knf services/api/*.toml -G '{defaults,prod}.toml'"
    )]
    glob_filename: Option<GlobPattern>,

    /// Print the file list in merge order without reading contents
    #[arg(short = 'l', long)]
    pub list_files: bool,

    /// Inline config, applied after all files
    #[arg(
        short = 'c',
        value_name = "KEY.PATH=VALUE",
        value_parser = parse_inline_config,
        long_help = "\
Inline config applied after all files; repeatable. The value is parsed in the
format of the input files or context, falling back to a string. With neither,
--to selects the inline format.

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

    /// Convert the merged document to this output format
    #[arg(
        long,
        value_name = "FORMAT",
        long_help = to_long_help()
    )]
    pub to: Option<OutputFormat>,

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
an object from another config. --shallow forces replacement.

  data_dir = \"${root}/data\"
  port     = \"${env:PORT}\""
    )]
    pub interpolate: bool,

    /// Read an interpolation context without merging it into output
    #[arg(
        long = "with",
        value_name = "FILE",
        requires = "interpolate",
        long_help = "\
Read one context file for interpolation; requires --interpolate. Each complete
reference path is looked up in the merged document first, then in the context.
Context values and their dependencies resolve only when referenced. Selected
containers keep their own children. The context uses the same format as inputs.

  knf foo.toml bar.toml -i --with config.toml"
    )]
    pub with: Option<PathBuf>,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,

    /// Run a command after -- with the merged file in KNF_CONFIG_PATH
    #[arg(
        long,
        conflicts_with = "list_files",
        long_help = "\
Run the command and arguments after --. KNF_CONFIG_PATH points to a private
temporary JSON or TOML file containing the merged configuration. The file is
removed when the command finishes. The command inherits standard streams and
its exit status is returned. With --to, the file is in the converted format.
Cannot be combined with --list-files.

  knf base.toml -c seed=42 --exec -- python fit.py --replicate 2"
    )]
    exec: bool,
}

/// A command line whose options have been checked against each other.
pub struct Invocation {
    pub cli: Cli,
    /// `--glob` or `--glob-filename`.
    pub filter: Option<(GlobPattern, GlobTarget)>,
    /// The output `--to` asks for; without it, output follows the inputs.
    pub to: Option<Emit>,
    pub exec: Option<Exec>,
}

/// The `--exec` child.
pub struct Exec {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// The snapshot format `--to` asks for; without it, that of the inputs.
    pub format: Option<InputFormat>,
}

fn usage_error(kind: ErrorKind, message: &str) -> ! {
    Cli::command().error(kind, message).exit()
}

impl Cli {
    /// Splits the child command (everything after the first `--`) off argv when
    /// `--exec` precedes that `--`, otherwise leaves argv for clap. Exits with a
    /// usage error on options that cannot be combined, or if `--exec` has no
    /// command.
    pub fn parse_command(args: impl IntoIterator<Item = OsString>) -> Invocation {
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
        let mut cli = Self::parse_from(args);

        let layout = Layout::pretty(!cli.compact);
        let to = cli.to.map(|to| {
            Emit::new(to.into(), layout).unwrap_or_else(|_| {
                usage_error(
                    ErrorKind::ArgumentConflict,
                    "--to yaml cannot be combined with --compact",
                )
            })
        });
        let exec = cli.exec.then(|| {
            let format = cli.to.map(|to| {
                to.snapshot_format().unwrap_or_else(|| {
                    usage_error(
                        ErrorKind::ArgumentConflict,
                        "--to yaml cannot be combined with --exec",
                    )
                })
            });
            let mut command = command.into_iter();
            let Some(program) = command.next() else {
                usage_error(
                    ErrorKind::MissingRequiredArgument,
                    "--exec requires -- followed by a command",
                )
            };
            Exec {
                program,
                args: command.collect(),
                format,
            }
        });
        // clap rejects the two globs together.
        let path = cli.glob.take().map(|glob| (glob, GlobTarget::Path));
        let filter = path.or_else(|| {
            let glob = cli.glob_filename.take();
            glob.map(|glob| (glob, GlobTarget::Filename))
        });

        Invocation {
            cli,
            filter,
            to,
            exec,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_values_follow_the_output_format_where_it_can_be_read() {
        assert_eq!(OutputFormat::Json.inline_format(), InputFormat::Json);
        assert_eq!(OutputFormat::Toml.inline_format(), InputFormat::Toml);
        #[cfg(feature = "yaml")]
        assert_eq!(OutputFormat::Yaml.inline_format(), InputFormat::Json);
    }

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
