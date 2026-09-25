# knf

Merges layered configuration files and prints the result. One job, no query
language, no template engine.

```bash
# Print the output to stdout
knf base.toml prod.toml > merged.toml
# Add manual overrides via the --set flag
knf defaults.json overrides.json --set server.port=8080 --set host=name
# Mix toml and json (if you want)
knf *.toml *.json
```

It exists because more powerful alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

## Installation

```bash
pip install pyknf      # `from knf import deep_merge`, and the `knf` executable
cargo install knf-cli  # the executable alone, without Python
```

`pyknf` is the [Python](#python) module and the `knf` command line. Wheels are
published for Linux (glibc and musl) on x86-64 and ARM64, macOS on Intel and
Apple Silicon, and Windows on x64 and ARM64; it needs CPython 3.9+. No Rust
toolchain is needed to install a wheel.

## Python

`pip install pyknf`. `deep_merge` runs the same merge as the command line, natively, and returns a
`dict`:

```python
from knf import deep_merge

config = deep_merge(["base.toml", "prod.json"], override={"server": {"port": 8080}})
```

Set `interpolate=True` to resolve `${key.path}` references against the final
merged config, including nested keys and array elements such as
`${servers[0].host}`. A whole-string reference keeps its value's type (so
`"${server.port}"` can become an integer and `"${server}"` a dict); an embedded
reference such as `"http://${server.host}:${server.port}/"` becomes text.
`${env:NAME}` reads the process environment, using the CLI's JSON-or-string
typing for whole-string references and raw text when embedded. Use `$$` for a
literal `$`. Interpolation is off by default. Invalid or missing references
and cycles raise `knf.InterpolationError`, a `ValueError`, with key paths.

Files are merged left to right, exactly like `knf base.toml prod.json`. If you pass
`override`, it is merged last as one more layer, which makes it the Python version
of `--set`. It must be a `dict` of JSON-like values: `None`, `bool`, `int` (within
64 bits), `float`, `str`, `list`/`tuple` and nested `dict`s with `str` keys, at most
128 levels deep and without cycles. Anything else raises `TypeError` or `ValueError`
naming the key path, before any file is read.

A file that can't be read raises `FileNotFoundError`, `PermissionError` or
`IsADirectoryError`, as `open()` would. Invalid JSON or TOML raises
`knf.ParseError`, a `ValueError` like `json.JSONDecodeError`. TOML datetimes come
back as their TOML spelling in a `str`, which is also what `knf -f json` prints.

## Rust library

The whole pipeline (read paths, parse JSON and TOML, merge, interpolate) is
`knf-core`. It's published separately from the command line, so a Rust consumer or
a language binding never pulls in `clap`:

```bash
cargo add knf-core
```

The library is named `knf`. Loading, merging and interpolating are three
functions, and you compose them:

```rust
use knf::{MergeOptions, load_layers, merge};

let (layers, _formats) = load_layers(&["base.toml", "prod.toml"], None)?;
let merged = merge(layers, &MergeOptions::default())?;
```

`load_layers` infers each file's format from its extension; pass `Some(Format::…)`
to override (required for `-`, which reads stdin). It also returns the format
each file was read as, so you can pick an output format before merging.

`MergeOptions` sets strict mode and shallow merge. `merge` takes any list of
`knf::Value`s, so in-memory overlays are just more layers appended after the
files. An overlay should be a `Value::Object`: a scalar layer replaces the whole
document instead of shadowing a key. The result is the format-independent
`knf::Value`, ready for a native adapter or language binding to convert without
parsing rendered stdout. `knf::format::emit` renders it when you do want text.

Interpolation is a separate, opt-in step, run once on the merged document.
Supply the environment yourself, or use `knf::ProcessEnv` for the real one:

```rust
let merged = knf::interpolate(merged, &knf::ProcessEnv)?;
```

Errors are typed rather than prose (`LoadError`, `MergeError`, `InterpError`,
`TomlError`) and never name a command-line flag, since a library caller has no
command line to act on. A null reaching TOML, for instance, is reported by the
paths it was found at. Whether the remedy is spelled `-f json` is up to your
interface, not the library.

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
  a byte-level no-op.

`--strict` errors when a layer changes the *type* of an existing key, which
catches the class of mistake where a leaf accidentally shadows a subtree.

```
$ knf a.json b.json --strict
error: type conflict at `server`: object would be replaced by number
```

### Shallow merge

The default is a deep merge. `--shallow` merges top-level keys only: a later
layer's value replaces the earlier one whole, so keys it omits are dropped.
These are jq's two object operators:

| knf | jq | `{"db":{"host":"a","port":1}}` then `{"db":{"host":"b"}}` |
| --- | --- | --- |
| `knf a.json b.json` | `a * b` | `{"db":{"host":"b","port":1}}` |
| `knf a.json b.json --shallow` | `a + b` | `{"db":{"host":"b"}}` |

Arrays replace wholesale in both, exactly as in jq; nothing is ever
concatenated. `--set` layers are ordinary layers, so `--shallow --set
db.host=x` leaves `db` with nothing but `host`.

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
without the flag the output is byte for byte what it is today.

Where the reference sits decides what it yields:

| Position | Behaviour |
| --- | --- |
| whole string — `port = "${p}"` | takes the referent's **value and type**; `port` above is a number, and `"${db}"` is the whole table |
| embedded — `url = "x/${p}"` | stringifies; an object or array has no format-independent spelling here, so it is an error |

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

## Caveats with formats

JSON and TOML, inferred from the file extension. `--input-format` overrides it
for every input and is required for `-` (stdin).

Output is the inputs' format when they agree; when they don't, `-f` is required
rather than guessed, so reordering arguments can never silently change the
encoding. Pretty-printed by default; `--compact` opts out.

A TOML datetime is a distinct type all the way through the merge, so every TOML
output keeps it unquoted — including a merge that mixed in a JSON layer, and
including `--set` on top. It becomes a plain string only under `-f json`, where
there is nothing else it could be.

TOML cannot represent null, so emitting TOML from a document containing one is
an error that names every path:

```
$ knf base.toml override.json -f toml
error: cannot serialize null to TOML
  --> servers.primary.proxy
  --> logging.sink
help: emit JSON with -f json, substitute with --null-as, or remove the null
```

Alternatively, you may use `--null-as <string>` to parse nulls into a custom value:

```bash
knf base.toml override.json -f toml --null-as=none
```
The option is a no-op for JSON output. 

Two more values have no spelling in one format or the other, and both are
rejected the same way — named by path, never silently substituted.

TOML integers are signed 64-bit, so an ID above `i64::MAX` (a snowflake, a hash)
round-trips exactly through JSON but cannot be written as TOML at all:

```
$ knf ids.json -f toml
error: cannot serialize integer to TOML
  --> id: `10000000000000000001`
help: TOML integers are signed 64-bit; emit JSON with -f json
```

Conversely, TOML's number grammar has `inf`, `-inf` and `nan` literals and
JSON's has none of them:

```
$ knf limits.toml -f json
error: cannot serialize non-finite number to JSON
  --> timeout: `inf`
help: emit TOML with -f toml, which can represent inf and nan
```

Each format is the escape from the other's rejection, and no same-format
round-trip is affected: `knf ids.json -f json` and `knf limits.toml -f toml`
both emit their input unchanged.

## Testing

```bash
cargo test --workspace
cargo test -p knf-core --lib     # fast inner loop: unit tests only
```

## License

MIT — see [LICENSE](LICENSE).
