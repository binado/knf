//! clap derive structs.

use std::path::PathBuf;

use clap::Parser;
use knf::{Format, PathLeaf};

/// `-f` and `--input-format`, as clap sees them.
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
  knf base.toml prod.toml --shallow            # top-level keys only

Objects merge key by key, recursively. Arrays, scalars and null all replace
wholesale — null is an ordinary value that overwrites, not a delete
instruction. This is jq's `a * b`; --shallow gives jq's `a + b`."
)]
pub struct Cli {
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
    pub input_format: Option<FormatArg>,

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
  proxy=null      -> null     (an error under -f toml, like any other null)
  tags=[\"a\",\"b\"]  -> array
  tags=[a,b]      -> \"[a,b]\"  (not valid JSON, so a string)

Sharp edge: version=1.0 is the number 1.0, not the string \"1.0\". Force a string
by quoting into JSON: --set version='\"1.0\"'.

Dotted paths nest, so keys containing a literal dot are not addressable from
--set; use a file. Brackets name array elements only in ${...} references, so
--set 'a[0]=1' is an error rather than a write into an array or to a key
literally spelled a[0]; only a file can carry either."
    )]
    pub set: Vec<PathLeaf<String>>,

    /// Merge top-level keys only, replacing each value whole
    #[arg(
        long,
        long_help = "\
Merge top-level keys only, replacing each value whole.

The default is a deep merge, like jq's `a * b`: objects recurse key by key.
--shallow is jq's `a + b`: a later layer's value for a top-level key replaces
the earlier one entirely, so keys it omits are dropped:

  knf base.toml prod.toml --shallow    # [db] is prod's [db], entirely

Arrays replace wholesale either way; nothing is ever concatenated.

This applies to --set layers too, which are ordinary layers: --shallow
--set db.host=x leaves db with nothing but host."
    )]
    pub shallow: bool,

    /// Output format; required when inputs are mixed
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<FormatArg>,

    /// Write this string in place of null when emitting TOML
    #[arg(
        long,
        value_name = "STRING",
        long_help = "\
Write this string in place of null when emitting TOML.

TOML has no null, so a null reaching TOML output is an error by default. This
substitutes a value of your choosing instead:

  knf base.toml override.json -f toml --null-as=none

It applies to TOML output only. JSON can hold a null, so under -f json the flag
has nothing to rescue and is ignored rather than corrupting a document that was
never in trouble.

The substitution writes a value that appeared in none of the inputs, which is
why it is opt-in and why the string is yours to pick. It also applies inside
arrays, where a null cannot simply be dropped without shifting every index
after it."
    )]
    pub null_as: Option<String>,

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
