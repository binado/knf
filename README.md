# knf

Merges layered configuration files and prints the result. One job, no query
language, no template engine. Each run merges JSON layers or TOML layers and
writes that same format.

```bash
# Print the output to stdout
knf base.toml prod.toml > merged.toml
# Add manual overrides via the --set flag
knf defaults.json overrides.json --set server.port=8080 --set host=name
# Choose one format for stdin or inline-only layers
knf -f toml --set server.port=8080
```

It exists because more powerful alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

## Installation

```bash
pip install pyknf      # `from knf import load`, and the `knf` executable
cargo install knf-cli  # the executable alone, without Python
```

`pyknf` is the [Python](#python) module and the `knf` command line. Wheels are
published for Linux (glibc and musl) on x86-64 and ARM64, macOS on Intel and
Apple Silicon, and Windows on x64 and ARM64; it needs CPython 3.9+. No Rust
toolchain is needed to install a wheel.

## Python

`pip install pyknf`. `load` runs the same merge as the command line, natively, and returns a
`dict`:

```python
from knf import load

config = load(["base.toml", "prod.toml"])
config["server"]["port"] = 8080
```

Discover and filter inputs separately before loading them:

```python
from knf import accumulate, filter_paths, load

files = accumulate("services/api/prod.toml", base_dir=project_root)
files = filter_paths(files, "{defaults,prod}.toml", filename_only=True)
config = load(files, interpolate=True)
```

Both helpers return `list[pathlib.Path]` and accept strings or `os.PathLike`
objects. `accumulate` uses the [same discovery rules as the CLI](#accumulating-layers-from-a-target-path).
Its target must be relative, even with `base_dir`. Without a base, results are
relative to the working directory; an explicit base (relative or absolute)
produces absolute paths without changing the working directory or resolving
symlinks. Discovery does not read configuration contents.

`filter_paths` filters an existing list without filesystem access. By default
the pattern matches the entire supplied path; `filename_only=True` matches just
its filename. It uses the [CLI's glob syntax](#filtering-inputs-with-globs),
preserving order and duplicates. Matching happens before conversion to `Path`;
returned `Path` objects normalize components such as `./`, so later filtering
sees the normalized spelling. An empty selection is allowed.

Invalid targets or patterns and other non-regular targets raise `ValueError`.
Discovery filesystem failures raise `OSError` subclasses with `.errno` and
`.filename` set, including `IsADirectoryError` for a directory target.

Set `interpolate=True` to resolve `${key.path}` references against the final
merged config, including nested keys and array elements such as
`${servers[0].host}`. A whole-string reference keeps its value's type (so
`"${server.port}"` can become an integer and `"${server}"` a dict); an embedded
reference such as `"http://${server.host}:${server.port}/"` becomes text.
`${env:NAME}` reads the process environment, using the selected format's value-or-string
typing for whole-string references and raw text when embedded. Use `$$` for a
literal `$`. Interpolation is off by default. Invalid or missing references
and cycles raise `knf.InterpolationError`, a `ValueError`, with key paths.

Files are merged left to right, exactly like `knf base.toml prod.toml`. Make
additional changes to the returned `dict` in Python, including nested updates
such as `config["server"]["port"] = 8080`.

A file that can't be read raises `FileNotFoundError`, `PermissionError` or
`IsADirectoryError`, as `open()` would. Invalid JSON or TOML raises
`knf.ParseError`, a `ValueError` like `json.JSONDecodeError`. All files must use
one format; mixed JSON/TOML inputs raise `ValueError` before contents are read.
An empty file list returns `{}`.

TOML offset and local datetimes return `datetime.datetime` objects (with a fixed
UTC offset or no timezone); dates return `datetime.date`, and local times return
`datetime.time`, as in Python's `tomllib`. Fractional seconds truncate to
microseconds at the Python boundary. A datetime Python cannot represent, such
as year zero, raises `ValueError` with its key path. The Rust/CLI pipeline keeps
TOML's native nanosecond precision.

## Rust library

The whole pipeline (read paths, parse JSON and TOML, merge, interpolate) is
`knf-core`. It's published separately from the command line, so a Rust consumer or
a language binding never pulls in `clap`:

```bash
cargo add knf-core
```

The library is named `knf`. The loader returns homogeneous native layers;
match the format once and compose the generic functions:

```rust
use knf::{Layers, MergeOptions, load_layers, merge};

let Layers::Toml(layers) = load_layers(&["base.toml", "prod.toml"], None)? else {
    unreachable!("TOML inputs");
};
let merged = merge(layers, &MergeOptions::default())?; // toml::Value
let merged = knf::interpolate(merged, &knf::ProcessEnv)?; // optional
let text = knf::format::emit(merged, true)?;
```

`ConfigValue` and `ConfigObject` provide structural operations for the shared
merge algorithm. `ConfigFormat` adds parsing, inline typing and serialization
for `serde_json::Value` and `toml::Value`; interpolation uses it to type
whole-string environment references. No combined IR or format conversion is
involved. In-memory overlays are native objects/tables appended to the flat
layer list. `PathLeaf<String>::into_layer::<V>()` builds an inline layer in the
selected native format.

`MergeOptions` sets strict mode and an optional `knf::glob::KeyGlobPattern`
selecting full key paths for wholesale replacement. For example,
`MergeOptions { shallow: Some("db.*".parse()?), ..Default::default() }` merges
inside `db` shallowly. `MergeOptions::shallow_root()` uses `*`, equivalent to
`--shallow '*'`. All numbers share one strict kind; TOML datetimes are distinct
from strings. Arrays replace wholesale; JSON null overwrites as an ordinary value.

`load_layers` infers one format from all extensions; pass `Some(Format::…)` to
override parsing for every input (required for stdin `-`). Empty input returns
`Layers::Json`, unless explicitly overridden. `resolve_format` performs the same
selection without reading document contents. Mixed inferred formats raise
`LoadError::MixedFormats` before document parsing.

`knf::fs` exposes discovery and filtering independently of loading:

```rust
use std::path::{Path, PathBuf};
use knf::fs::{AccumulateTarget, GlobPattern, accumulate, filter_paths};

let target = AccumulateTarget::try_from(PathBuf::from("services/api/prod.toml"))?;
let files = accumulate(&target, Some(Path::new("project")))?;
let pattern: GlobPattern = "{defaults,prod}.toml".parse()?;
let files = filter_paths(&files, &pattern, true); // filename-only matching
let layers = knf::load_layers(&files, None)?;
```

Pass `None` as the base for working-directory-relative results, or `Some(base)`
for absolute results. Target validation, glob validation and discovery failures
are typed (`AccumulateTargetError`, `GlobError`, `AccumulateError`); discovery
failures carry paths and underlying I/O errors for frontend diagnostics.

Interpolation runs once after merging. Supply an `Env` implementation whose
`lookup` returns raw `Option<String>`, or use `ProcessEnv` for the real process
environment. Embedded references insert raw text; whole-string references type
through the native adapter. Errors (`LoadError`, `MergeError`, `InterpError`)
remain typed and never name command-line flags.

## Merging

Files are merged left to right in argument order. Exactly one document
goes to stdout.

| Case | Behaviour |
| --- | --- |
| object ⊕ object | recurse per key |
| array ⊕ anything | **replace wholesale**, never index-merge or concat |
| scalar ⊕ anything | last wins |
| anything ⊕ null | null is an ordinary value; it overwrites |

Two consequences worth knowing:

- **Arrays replace**, always. Index-merging would turn `["a"]` over
  `["x","y","z"]` into `["a","y","z"]` — a value nobody wrote.
- **Null is a value, not a delete.** So `knf a.json` with one argument is always
  unchanged as a value. Serialization may change whitespace.

`--strict` errors when a layer changes the *type* of an existing key, which
catches the class of mistake where a leaf accidentally shadows a subtree.

```
$ knf a.json b.json --strict
error: type conflict at `server`: object would be replaced by number
```

### Filtering inputs with globs

`-g`, or `--glob`, filters the whole input path. `-G`, or `--glob-filename`,
filters only the filename, ignoring directories. Each accepts one pattern;
they cannot be repeated or combined. Quote the pattern so knf receives it
without shell expansion:

```bash
knf config/**/*.toml -g 'config/**/prod.toml'
knf config/**/*.toml -G '*.prod.toml'
knf -a services/api/prod.toml -G '{defaults,prod}.toml'
```

The positional glob in the first two examples is expanded by your shell. knf's
filter does not discover files: it removes inputs from the positional list or
the list produced by `--accumulate`, preserving order, spelling and duplicates.
Filtering happens before reading configuration contents, so excluded positional
inputs need not exist or parse successfully. Format inference uses only
retained inputs. `--list-files` shows the filtered list.

Matching is case-sensitive and covers the entire path or filename: `.prod.toml`
matches that exact name, while `*.prod.toml` matches names ending in `.prod.toml`.
Patterns support `*`, `?`, character classes such as `[a-z]`, alternatives such
as `{defaults,prod}.toml`, and leading `!` negation. `*` does not cross `/`;
`**` can cross directories when it occupies a complete path segment. Matching
uses native encoded bytes without replacement characters; `?` matches one byte,
not a Unicode character. Malformed patterns are usage errors (exit code 2).

Whole-path matching preserves lexical components such as `./`; on Windows,
path separators match as `/`, without altering paths used for I/O or listing.
Filename matching excludes paths without a filename. Stdin's `-` is matched
literally in both modes; include it with a pattern such as `{*.toml,-}`.

An empty selection is allowed: the result is an empty object, or only the
`--set` layers when supplied. `--list-files` prints nothing for an empty list.
With `--accumulate`, the filter can also exclude the named target; discovery
still checks that it exists and inspects directories before filtering, so
discovery inspection errors are not suppressed.

### Accumulating layers from a target path

`-a`, or `--accumulate`, is a discovery operator: it expands one relative target
path into an ordered list of file inputs, then processes them exactly like
explicit file arguments:

```text
cwd/
  foo/
    conf1.toml
    conf2.toml
    bar/
      conf4.toml
```

From `cwd`, these commands merge the same layers:

```bash
knf -a foo/bar/conf4.toml
knf foo/conf1.toml foo/conf2.toml foo/bar/conf4.toml
```

Discovery starts at the first directory (`foo`), excluding files directly in
`cwd`. It visits only directories on the target's path, from shallowest to
deepest, collecting matching regular files directly inside each one. Files in
other branches and directories with matching extensions are ignored. Empty
matching directories contribute no layers.

The target's extension selects JSON or TOML, case-insensitively. Only files of
that format are discovered, including hidden files. `-f` overrides
how those files are parsed and emitted; it does not change which files are selected.

Files within each directory are sorted by filename using native string order,
without locale or numeric sorting. Other matching files in the target's
directory are included. The named target is included exactly once, **last**,
even if its filename sorts first. A glob filter can then remove any layer,
including the target; the target stays last if retained. Symlinks follow
ordinary filesystem semantics; different filenames pointing to the same file
are separate layers. The supplied
path controls directory selection, rather than enforcing filesystem containment.

Exactly one target is required, with a JSON or TOML extension. Stdin, absolute
paths and `..` components are rejected. `./foo/bar/conf4.toml` is normalized to
`foo/bar/conf4.toml`. A target directly in `cwd`, such as `-a conf4.toml`, merges
only that file. Missing or non-file targets and filesystem inspection errors
fail rather than silently skipping files.

Inspect the file list without reading configuration contents:

```bash
knf foo/conf1.toml foo/conf2.toml foo/bar/conf4.toml --list-files
# foo/conf1.toml
# foo/conf2.toml
# foo/bar/conf4.toml

knf -a foo/bar/conf4.toml --list-files
# foo/conf1.toml
# foo/conf2.toml
# foo/bar/conf4.toml
```

`-l`, or `--list-files`, prints one path per line and exits without parsing files, merging,
interpolating or emitting a configuration. The list is the positional files, or
the files `--accumulate` discovered, after any glob filter. Discovery still
checks that the target exists and is a regular file.
As elsewhere in diagnostic output, filenames that are not UTF-8 are displayed
with replacement characters; file operations preserve the original names.

Normal merging flags work with accumulate mode: `--set` layers apply after all
files, `--strict` and `--shallow` use the discovered order, interpolation runs
once on the merged result, and `-f` selects the native format for parsing and
output. The Rust `knf::fs` module and Python `accumulate`/`filter_paths` helpers expose the same
discovery and filtering; `load_layers` and Python `load` take the resulting
explicit file lists.

### Inline overrides

Repeat `--set KEY.PATH=VALUE` to append layers after all files, in occurrence
order. Values parse in the selected format; invalid or out-of-range literals
fall back to their original text as strings. Surrounding spaces, tabs and line
breaks are ignored when parsing a literal; quoted string contents and string
fallbacks retain their whitespace.

| RHS | JSON | TOML |
| --- | --- | --- |
| `8080`, `true`, `"name"` | number, bool, string | number, bool, string |
| `null` | null | string `"null"` |
| `["a","b"]` | array | array |
| `{host="local"}` | string | inline table |
| `1979-05-27` | string | datetime |
| `inf` | string | non-finite float |
| `18446744073709551615` | exact integer | string (outside TOML's integer range) |

`version=1.0` is a number. Force a string with `--set version='"1.0"'`.
With no retained file inputs, JSON is the default; use `-f toml` to select TOML.
Writable paths contain keys only, never array indices.

### Shallow merge

The default is a deep merge. `--shallow PATTERN` selects full key paths whose
values replace wholesale, without recursing into their descendants. Use `*`
to replace top-level values, giving jq's shallow `a + b`:

| knf | jq | `{"db":{"host":"a","port":1}}` then `{"db":{"host":"b"}}` |
| --- | --- | --- |
| `knf a.json b.json` | `a * b` | `{"db":{"host":"b","port":1}}` |
| `knf a.json b.json --shallow '*'` | `a + b` | `{"db":{"host":"b"}}` |

`foo` replaces the entire value at `foo`, dropping children omitted by the later
layer. `foo.*` replaces its immediate children, preserving children omitted by
the later layer and keeping siblings deep:

| knf | `{"db":{"pool":{"min":1,"max":5},"host":"a"},"app":{"x":1}}` then `{"db":{"pool":{"max":9}},"app":{"y":2}}` |
| --- | --- |
| `knf a.json b.json --shallow 'db'` | `{"db":{"pool":{"max":9}},"app":{"x":1,"y":2}}` |
| `knf a.json b.json --shallow 'db.*'` | `{"db":{"pool":{"max":9},"host":"a"},"app":{"x":1,"y":2}}` |

The flag accepts exactly one nonempty pattern and cannot be repeated. Both
`--shallow PATTERN` and `--shallow=PATTERN` work. Bare `--shallow`, `--shallow=`,
and malformed patterns are usage errors (exit code 2), before discovery or
file I/O. Quote patterns to prevent shell expansion.

Matching is case-sensitive against the **full key path**. Dots separate keys;
`*` stays within one key and `**` crosses keys when it occupies a complete
segment. `?` matches one UTF-8 byte, not one Unicode character. Character
classes, ranges, brace alternatives, and leading `!` negation use the same
syntax as [input globs](#filtering-inputs-with-globs):

```bash
knf a.json b.json --shallow '{db,cache}.*' # shallow inside both objects
knf a.json b.json --shallow '**.cache'    # replace cache at any depth
```

Single-quoted spans are literal, including dots and glob metacharacters. The
inner quotes must reach knf; shell quoting alone does not make a key literal:

```bash
knf a.json b.json --shallow "foo.bar"     # nested foo -> bar
knf a.json b.json --shallow "'foo.bar'"   # one literal key foo.bar
knf a.json b.json --shallow "'foo.bar'.*" # children of that literal key
knf a.json b.json --shallow "'*'"         # one literal key named *
```

Inside quoted spans, backslash escapes the next character literally, including
single quotes and backslashes. Outside quotes, glob escaping applies (including
`\n`, `\r`, `\t`, and `\b`); `foo\.bar` also selects the literal key `foo.bar`.
Dots inside character classes are literal. Slashes are always literal key
characters, so `foo/bar` selects one key and `foo.bar` selects two segments.
Literal slash and backslash keys match consistently across platforms.
`''` selects an empty key. Quoted spans may occur inside brace alternatives or
next to unquoted pattern text. Double quotes have no special meaning to knf.

A matching ancestor stops traversal, making selectors below it moot. Negation
applies to each visited full path: `!foo.*` matches `foo` itself, replacing it
whole before its children are visited. A missing path changes nothing. Arrays
are never traversed; brackets are glob character classes, not array indices.
Use a quoted segment such as `'servers[0]'` to select a literal bracketed key.
`--set` and interpolation references retain their existing path grammar.

Strict mode checks kinds at replacement boundaries without inspecting replaced
descendants. Arrays replace wholesale either way; null overwrites normally.
`--set` layers are ordinary layers, so `--shallow '*' --set db.host=x` leaves
`db` with nothing but `host`. Interpolation still runs once after the merge.

Migration from the previous path-list API: bare or empty `--shallow` becomes
`--shallow '*'`; old `--shallow=foo` becomes `--shallow 'foo.*'`; multiple paths
such as `--shallow=foo --shallow=bar` become `--shallow '{foo,bar}.*'`.

### Variable and environment references

A merged config often wants to refer to itself, or to the environment.
`--interpolate` resolves `${key.path}` and `${env:VAR}` in string values, in one
pass over the merged document:

```toml
# base.toml
root     = "/srv"
data_dir = "${root}/data"
port     = "${env:PORT}"
url      = "http://localhost:${env:PORT}/health"
literal  = "$${NOT_A_REF}"
```

```console
$ PORT=8080 knf base.toml --interpolate
root = "/srv"
data_dir = "/srv/data"
port = 8080
url = "http://localhost:8080/health"
literal = "${NOT_A_REF}"
```

**It is opt-in, and off by default.** knf sits directly upstream of tools whose
own syntax is `${...}` — compose files, GitHub Actions workflows, Helm charts,
systemd units. Eating those without being asked would be silent corruption, so
without the flag the reference strings pass through unchanged.

Where the reference sits decides what it yields:

| Position | Behaviour |
| --- | --- |
| whole string — `port = "${p}"` | takes the referent's **value and type**; `port` above is a number, and `"${db}"` is the whole table |
| embedded — `url = "x/${p}"` | stringifies; objects, arrays and JSON null are errors in embedded positions |

Embedded finite floats keep their existing Rust float spelling, including
`1.0`, `1e20` and `1e-7`. Serializers may spell the same number differently:
JSON emits `1e+20`, and TOML emits `100000000000000000000.0`. Whole-string
references retain the native numeric value and use the selected serializer.

An environment variable is typed by the same rule as `--set`'s right-hand side
when it is the whole string, and spliced as raw text when it is embedded —
parsing it only to print it again could only lose something.

`$$` is a literal `$`. A `$` followed by anything else is ordinary text, so
`USD $5` needs no escaping.

Document references resolve transitively and in any order; environment values
are terminal and are never re-scanned. Cycles are an error, and so is a
reference that names nothing:

```
$ knf base.toml --interpolate
error: unresolved reference
  --> server.url: `db.hostname`
  --> tags[0]: `env:REGION`
help: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable
help: drop --interpolate to pass `${...}` through untouched
```

A reference may also read an array element — `${servers[0].host}` — with the
same two-position rules: whole-string it takes the element's value and type,
embedded it stringifies.

Two limits worth knowing:

- **`env:` is a reserved prefix**, matched literally rather than by splitting on
  the first `:`. So `${a:b}` is the ordinary key `a:b`, and only keys that
  literally begin `env:` are unaddressable.
- **A key spelled with brackets is unaddressable** — `${a[0]}` now reads as *the
  first element of `a`*, never as a key literally named `a[0]`, and
  `--set 'a[0]=1'` is an error rather than a write into an array. Only a file
  can carry such a key. The same accepted loss as keys containing a literal
  dot, which the dotted grammars have always excluded.

`--set` layers interpolate like any other layer. `--strict` runs during the
merge, before any substitution, so it compares the types values had when they
were written.

## Formats

Each invocation uses one native format: JSON or TOML. Without `-f/--format`,
infer it from the retained file extensions (case-insensitively). Mixed formats
are rejected before reading document contents:

```text
$ knf base.toml override.json
error: inputs mix JSON and TOML formats; layers must use one format
help: merge JSON layers and TOML layers separately
```

`-f` selects parsing, inline/environment typing and output together. It overrides
all input extensions and is required for stdin:

```bash
knf base.json - -f json
knf config.data -f toml
knf -f toml --set server.port=8080
```

It never converts formats: `knf config.toml -f json` attempts to parse the file
as JSON and fails when its contents are TOML. `--list-files` exits before format
selection. With no retained files or format option, output is JSON. Output is
pretty-printed by default; `--compact` opts out. Every output ends in a newline;
an empty TOML document is a newline.

JSON nulls and large unsigned integers remain native JSON values. TOML dates,
times, nanosecond precision and non-finite floats remain native TOML values.
There are no cross-format representability checks or substitutions. This
preserves parsed value semantics, not source formatting or comments.

### Migrating from the conversion pipeline

| Previous behavior/API | Replacement |
| --- | --- |
| Mixed JSON/TOML layers | Merge one format per invocation or Python `load()` call |
| `--input-format FORMAT` | `-f/--format FORMAT` |
| `-f` selecting a different output encoding | `-f` selects the entire native pipeline; use a separate conversion tool if needed |
| `--null-as` | Removed; TOML `--set proxy=null` now produces the string `"null"` |
| `knf::Value`, `Map`, `Number`, conversion helpers/errors | Native JSON/TOML values and `ConfigValue`/`ConfigObject` traits |
| `load_layers` returning values and format lists | `Layers::Json` or `Layers::Toml` |
| `EnvValue { raw, typed }` | `Env::lookup` returns raw `Option<String>` |
| `format::parse(format, …)` / `emit(value, format, …)` | Generic `parse::<V>(text, source)` / `emit(value, pretty)` |
| Python TOML datetime strings | Native Python date/time objects; fractional seconds truncate to microseconds |

## Testing

```bash
cargo test --workspace
cargo test -p knf-core --lib     # fast inner loop: unit tests only
```

## License

MIT — see [LICENSE](LICENSE).
