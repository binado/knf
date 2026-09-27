//! clap derive structs.

use std::path::PathBuf;
use std::str::FromStr;

use clap::{ArgAction, Parser};
use knf::{Format, PathError, PathLeaf, RefPath};

/// `-f`, as clap sees them.
///
/// A local mirror of [`Format`] rather than a derive on `Format` itself:
/// `knf-core` has no clap, and the orphan rule forbids implementing
/// `ValueEnum` for a foreign type from here. clap takes its possible values
/// from the variant idents, so `--help` reads `json`/`toml` either way.
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

/// Where one `--shallow` applies.
///
/// clap's derive cannot group values per occurrence, so a bare `--shallow`
/// arrives as its `default_missing_value`, the empty string, which is the
/// root — as the empty key path is in `MergeOptions`. `--shallow=` spells the
/// same thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShallowAt {
    Root,
    Path(RefPath),
}

impl FromStr for ShallowAt {
    type Err = PathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(Self::Root)
        } else {
            s.parse().map(Self::Path)
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "knf",
    version,
    about = "Merge layered configuration files and print the result",
    long_about = "\
Merge layered configuration files and print the result.

Files are layers, merged left to right in argument order. All inputs must use one
format: JSON or TOML. Exactly one document goes to stdout.

  knf base.toml prod.toml
  knf base.json - -f json          # stdin as a layer
  knf defaults.toml --set server.port=8080
  knf base.toml prod.toml --shallow            # top-level keys only
  knf base.toml prod.toml --shallow=db         # shallow inside db only

Objects merge key by key, recursively. Arrays, scalars and null all replace
wholesale — null is an ordinary value that overwrites, not a delete
instruction. This is jq's `a * b`; --shallow gives jq's `a + b`, at the root or
at a key path."
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
Filter the resolved inputs by a case-sensitive glob matching the whole path.
Quote the pattern to prevent shell expansion. Does not discover files, sort,
deduplicate or read excluded inputs. Matches positional inputs or the list
from --accumulate, including its target. --list-files shows the filtered list.

Preserves lexical components such as ./; Windows separators match as /.
Stdin is matched as the literal path -. An empty selection is allowed: only
--set layers remain, or an empty object if none were supplied.

Patterns support *, **, ?, character classes, braces and leading ! negation.
Matching uses native encoded bytes; ? matches one byte, not a Unicode character.
Invalid patterns are usage errors. Accepts one pattern and conflicts with
--glob-filename.

  knf config/**/*.toml -g 'config/**/prod.toml'"
    )]
    pub glob: Option<knf::fs::GlobPattern>,

    /// Filter inputs by a case-sensitive glob matching just the filename
    #[arg(
        short = 'G',
        long,
        value_name = "PATTERN",
        conflicts_with = "glob",
        long_help = "\
Like --glob, but match only the filename, ignoring directories. Inputs without
a filename do not match. Stdin is matched as the literal filename -.
Quote the pattern to prevent shell expansion. Accepts one pattern and conflicts
with --glob.

  knf config/**/*.toml -G '*.prod.toml'
  knf -a services/api/prod.toml -G '{defaults,prod}.toml'"
    )]
    pub glob_filename: Option<knf::fs::GlobPattern>,

    /// Accumulate same-format layers along one relative target path
    #[arg(
        short = 'a',
        long = "accumulate",
        value_name = "TARGET",
        value_parser = super::accumulate::parser(),
        conflicts_with = "files",
        long_help = "\
Accumulate same-format files along one relative target path, starting at its first
directory and excluding files directly in the working directory. Visit each
directory on the path in order; sort its matching files by filename. Include
files in the target's directory, then apply the named target exactly once, last
if retained by --glob or --glob-filename.

The target is this option's argument. It must have a JSON or TOML extension
(case-insensitive) and cannot be combined with positional files. -f overrides
parsing only, not discovery. --set layers still apply after all files.
Stdin, absolute paths and .. are not allowed.
Symlinks follow ordinary filesystem semantics.

  knf -a foo/bar/config.toml
  knf -a foo/bar/config.toml --list-files"
    )]
    pub accumulate: Option<knf::fs::AccumulateTarget>,

    /// Print the file list in merge order without reading contents
    #[arg(
        short = 'l',
        long,
        long_help = "\
Print the file list, one path per line, and exit.

The list is the positional files, or the files --accumulate discovered, after
--glob or --glob-filename filtering. An empty list prints nothing. Does not
read configuration contents, merge, interpolate or emit a configuration.
Discovery still checks filesystem access and that the target exists."
    )]
    pub list_files: bool,

    /// Inline terminal layer, applied after all files
    #[arg(
        long = "set",
        value_name = "KEY.PATH=VALUE",
        long_help = "\
Inline terminal layer, applied after all files. Repeatable; multiple --set apply
left to right.

The value is parsed in the selected format, falling back to the original text
as a string when that fails. JSON uses JSON literals; TOML uses TOML values:

  port=8080       -> number in both formats
  debug=true      -> bool in both formats
  name=foo        -> string in both formats
  proxy=null      -> null in JSON, string in TOML
  tags=[\"a\",\"b\"]  -> array in both formats
  db={host=\"a\"}  -> inline table in TOML, string in JSON
  day=1979-05-27  -> datetime in TOML, string in JSON

Invalid and out-of-range literals remain strings. version=1.0 is a number;
force a string with quotes: --set version='\"1.0\"'.

Dotted paths nest, so keys containing a literal dot are not addressable from
--set; use a file. Brackets name array elements only in ${...} references, so
--set 'a[0]=1' is an error rather than a write into an array or to a key
literally spelled a[0]; only a file can carry either."
    )]
    pub set: Vec<PathLeaf<String>>,

    /// Merge shallowly at the root, or at KEY.PATH; repeatable
    #[arg(
        long,
        value_name = "KEY.PATH",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "",
        action = ArgAction::Append,
        long_help = "\
Merge shallowly at the root, or at KEY.PATH. Repeatable.

The default is a deep merge, like jq's `a * b`: objects recurse key by key.
A shallow merge is jq's `a + b`: a later layer's value for each key replaces
the earlier one entirely, so keys it omits are dropped.

Bare --shallow merges the root shallowly. --shallow=KEY.PATH merges only the
object at that path shallowly; everything else stays deep:

  knf base.toml prod.toml --shallow       # [db] is prod's [db], entirely
  knf base.toml prod.toml --shallow=db    # db.pool is prod's db.pool, entirely;
                                          # db's other keys and all siblings
                                          # still merge deep

A path that is missing, or is not an object in both layers, changes nothing.
The `=` is required, so a bare --shallow never takes a filename as its path;
--shallow= (empty) is the root, like bare --shallow.

Arrays replace wholesale either way; nothing is ever concatenated.

This applies to --set layers too, which are ordinary layers: --shallow
--set db.host=x leaves db with nothing but host."
    )]
    pub shallow: Vec<ShallowAt>,

    /// Format for parsing, inline typing and output; required for stdin
    #[arg(
        short = 'f',
        long,
        value_name = "FORMAT",
        long_help = "\
Select the format for the entire pipeline: input parsing, --set and environment
typing, and output. Overrides every input extension; it never converts values.

Required for stdin (-). Without it, infer one format from retained input
extensions; mixed formats are rejected. No retained inputs default to JSON.
Use -f toml for TOML output with only --set layers."
    )]
    pub format: Option<FormatArg>,

    /// Resolve ${key.path} and ${env:VAR} references in the merged document
    #[arg(
        long,
        long_help = "\
Resolve ${key.path} and ${env:VAR} references in the merged document.

Opt-in, and off by default. knf sits upstream of tools whose own syntax is
${...} — compose files, GitHub Actions workflows, Helm charts, systemd units —
so eating those without being asked would be silent corruption. Off, the output
is exactly what it is today.

The pass runs once, on the merged document, never per layer:

  root     = \"/srv\"
  data_dir = \"${root}/data\"    -> \"/srv/data\"
  port     = \"${env:PORT}\"     -> 8080  (a number, not a string)
  url      = \"x:${env:PORT}\"   -> \"x:8080\"
  literal  = \"$${NOT_A_REF}\"   -> \"${NOT_A_REF}\"

A reference that is the *whole* string takes the referent's value and type, so
${port} can yield a number, an array or a table. A reference *embedded* in text
stringifies; an object or array has no format-independent spelling there, so it
is an error rather than a guess. An environment variable is spliced as raw text
when embedded and typed like --set's right-hand side when it is the whole
string.

$$ is a literal $. A $ followed by anything else is ordinary text, so `USD $5`
needs no escaping.

Document references resolve transitively and in any order; environment values
are terminal and are never re-scanned. Cycles are an error.

`env:` is a reserved prefix, matched literally: ${a:b} is the ordinary key
`a:b`, and only keys that literally begin `env:` are unaddressable.

A reference may read an array element — ${servers[0].host} — with all the same
rules: whole-string it takes the element's value and type, embedded it
stringifies. Brackets are part of the grammar, so a key literally spelled
`a[0]` cannot be addressed by a reference or written by --set, exactly as a
key containing a literal dot never could; only a file can carry one.

An unset variable or a missing key is an error naming every offender, never
passed through as literal text."
    )]
    pub interpolate: bool,

    /// Error when a layer changes the type of an existing key
    #[arg(long)]
    pub strict: bool,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `--shallow` occurrence as its rendered path; `""` is the bare flag.
    fn shallow(args: &[&str]) -> (Vec<String>, Vec<PathBuf>) {
        let cli = Cli::try_parse_from(std::iter::once("knf").chain(args.iter().copied()))
            .expect("argv parses");
        let paths = cli
            .shallow
            .iter()
            .map(|at| match at {
                ShallowAt::Root => String::new(),
                ShallowAt::Path(path) => path.to_string(),
            })
            .collect();
        (paths, cli.files)
    }

    #[test]
    fn shallow_occurrences_keep_the_bare_flag_as_the_root() {
        assert_eq!(shallow(&["a.json"]).0, Vec::<String>::new());
        assert_eq!(shallow(&["a.json", "--shallow"]).0, [""]);
        assert_eq!(shallow(&["a.json", "--shallow="]).0, [""]);
        assert_eq!(shallow(&["--shallow=db.pool"]).0, ["db.pool"]);
        assert_eq!(
            shallow(&["--shallow", "--shallow=db", "--shallow=x"]).0,
            ["", "db", "x"]
        );
    }

    /// Without `require_equals`, a bare `--shallow` would swallow the first file.
    #[test]
    fn bare_shallow_before_files_leaves_them_as_files() {
        let (paths, files) = shallow(&["--shallow", "a.json", "b.json"]);
        assert_eq!(paths, [""]);
        assert_eq!(files, [PathBuf::from("a.json"), PathBuf::from("b.json")]);
    }
}
