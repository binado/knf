//! clap derive structs.

use std::path::PathBuf;

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
  knf defaults.toml -c server.port=8080"
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

    /// Format for parsing, inline typing and output; required for stdin
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Resolve ${key.path} and ${env:VAR} references in the merged document
    #[arg(
        short = 'i',
        long,
        long_help = "\
Resolve ${key.path} and ${env:VAR} references in the merged document.

A whole-string reference keeps the value's type; an embedded one becomes text.
$$ is a literal $.

  data_dir = \"${root}/data\"
  port     = \"${env:PORT}\""
    )]
    pub interpolate: bool,

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

    /// Error when a layer changes the type of an existing key
    #[arg(long)]
    pub strict: bool,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,
}

impl Cli {
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
