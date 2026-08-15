//! clap derive structs.

use std::path::PathBuf;

use clap::Parser;
use knf_dotted::{KeyPath, PathLeaf};

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
  knf base.toml prod.toml --append plugins     # concatenate one array

Objects merge key by key. Arrays, scalars and null all replace wholesale —
null is an ordinary value that overwrites, not a delete instruction.

--append, --replace and --fail change that at the paths they name, and only
there. A path may be named by at most one of them, and since all three consume
the whole value at their path, no rule may sit below another. Rules are a set:
their order never affects the output."
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
  proxy=null      -> null     (an error under -f toml, like any other null)
  tags=[\"a\",\"b\"]  -> array
  tags=[a,b]      -> \"[a,b]\"  (not valid JSON, so a string)

Sharp edge: version=1.0 is the number 1.0, not the string \"1.0\". Force a string
by quoting into JSON: --set version='\"1.0\"'.

Dotted paths nest, so keys containing a literal dot are not addressable from
--set; use a file."
    )]
    pub set: Vec<PathLeaf<String>>,

    /// Concatenate arrays at this path instead of replacing them
    #[arg(
        long,
        value_name = "KEY.PATH",
        long_help = "\
Concatenate arrays at this path instead of replacing them. Repeatable.

Both sides must be arrays; anything else is an error. The path is only combined
where the merge already has a value for it, so a lone layer's array is inserted
as-is rather than doubled:

  knf base.toml prod.toml --append plugins    # base's plugins ++ prod's

Dotted paths address nested keys, so a key containing a literal dot cannot be
named."
    )]
    pub append: Vec<KeyPath>,

    /// Replace the value at this path wholesale, without merging into it
    #[arg(
        long,
        value_name = "KEY.PATH",
        long_help = "\
Replace the value at this path wholesale, without merging into it. Repeatable.

Object over object stops recursing, so the later layer's table is taken whole
and keys it omits are dropped:

  knf base.toml prod.toml --replace db        # db is prod's db, entirely

This applies to --set layers too, which are ordinary layers: --replace db
--set db.host=x leaves db with nothing but host.

Dotted paths address nested keys, so a key containing a literal dot cannot be
named."
    )]
    pub replace: Vec<KeyPath>,

    /// Error if a later layer sets this path again
    #[arg(
        long,
        value_name = "KEY.PATH",
        long_help = "\
Error if a later layer sets this path again. Repeatable.

The first layer to define the path pins it; the path may still be absent from
every layer. Use it to protect a value that later layers must not override:

  knf base.toml prod.toml --fail db.host

Dotted paths address nested keys, so a key containing a literal dot cannot be
named."
    )]
    pub fail: Vec<KeyPath>,

    /// Output format; required when inputs are mixed
    #[arg(short = 'f', long, value_name = "FORMAT")]
    pub format: Option<Format>,

    /// Write this string in place of null when emitting TOML
    #[arg(
        long,
        value_name = "STRING",
        long_help = "\
Write this string in place of null when emitting TOML.

TOML has no null, so a null reaching TOML output is an error by default. This
substitutes a value of your choosing instead:

  knf base.toml override.json -f toml --null-placeholder=none

It applies to TOML output only. JSON can hold a null, so under -f json the flag
has nothing to rescue and is ignored rather than corrupting a document that was
never in trouble.

The substitution writes a value that appeared in none of the inputs, which is
why it is opt-in and why the string is yours to pick. It also applies inside
arrays, where a null cannot simply be dropped without shifting every index
after it."
    )]
    pub null_placeholder: Option<String>,

    /// Error when a layer changes the type of an existing key
    #[arg(long)]
    pub strict: bool,

    /// Disable pretty-printing
    #[arg(long)]
    pub compact: bool,
}
