# knf

Merge layered JSON or TOML config files. No query language, no templates.

## Installation

### Python

```bash
pip install pyknf
```

Installs the `knf` module and the `knf` executable. Wheels for Linux, macOS and
Windows (x86-64 and ARM64); CPython 3.11+. The wheels include `--to yaml`.

### Rust

```bash
cargo install knf-cli                  # the executable
cargo install knf-cli --features yaml  # with `--to yaml` (Rust 1.89+)
cargo add knf-core knf-convert         # Rust libraries
```

Rust: `knf_convert::load_configs`, then `knf::merge_configs`, then
`Emit::emit`; see the [example](crates/knf-convert/examples/toml_to_json.rs).

## Usage

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json -c server.port=8080
knf -c server.port=8080 --to toml
```

Configs merge left to right. A file's extension (`.json` or `.toml`) selects its
format, and output defaults to the input format. There is no stdin input.

| Case | Behaviour |
| --- | --- |
| object ⊕ object | recurse per key |
| array ⊕ anything | replace wholesale |
| scalar ⊕ anything | last wins |
| anything ⊕ null | null overwrites (it is not a delete) |

## CLI

**`--dry-run`**: print a numbered list of files and inline assignments from
lowest to highest precedence without reading or merging configs.

```console
$ knf base.toml prod.toml -c server.port=8080 --dry-run
[dry run] Skipped merging configs in this order (lowest to highest precedence):
1. base.toml
2. prod.toml
3. server.port=8080
```

**`-c KEY.PATH=VALUE`**: inline config applied after all files. The value is
parsed in the format of the input files or context and falls back to a string.
With neither, `--to` selects the inline format (`yaml` parses as JSON).

| RHS | JSON | TOML |
| --- | --- | --- |
| `8080`, `true`, `"name"` | number, bool, string | number, bool, string |
| `null` | null | string `"null"` |
| `["a","b"]` | array | array |
| `{host="local"}` | string | inline table |
| `1979-05-27` | string | datetime |

Force a string with quotes: `-c version='"1.0"'`.

**`--shallow PATTERN`**: replace values at matching key paths wholesale instead
of merging them.

| knf | jq | `{"db":{"host":"a","port":1}}` then `{"db":{"host":"b"}}` |
| --- | --- | --- |
| `knf a.json b.json` | `a * b` | `{"db":{"host":"b","port":1}}` |
| `knf a.json b.json --shallow '*'` | `a + b` | `{"db":{"host":"b"}}` |

```bash
knf a.json b.json --shallow 'db'          # replace db entirely
knf a.json b.json --shallow '{db,cache}.*' # replace db's and cache's children
knf a.json b.json --shallow "'foo.bar'"   # single quotes: literal key foo.bar
```

**`-i/--interpolate`**: resolve `${key.path}` and `${env:VAR}` in the merged
document. Off by default, so `${...}` meant for other tools passes through.

```toml
# base.toml
root     = "/srv"
data_dir = "${root}/data"
port     = "${env:PORT}"
url      = "http://localhost:${env:PORT}/health"
literal  = "$${NOT_A_REF}"
```

```console
$ PORT=8080 knf base.toml -i
root = "/srv"
data_dir = "/srv/data"
port = 8080
url = "http://localhost:8080/health"
literal = "${NOT_A_REF}"
```

A whole-string reference keeps the value's type; an embedded one becomes text.
`$$` is a literal `$`.

A reference body is interpolated first, so it can select a subtree; inner
values must be scalars:

```toml
db = "${databases.${env:STAGE}}"   # STAGE=prod reads databases.prod
```

References bind to the final document, and a whole-string reference merges
exactly as the value it names:

```toml
# base.toml                     # override.toml
[bar]                           [foo.b]
a = 1                           a = 4
d = 1

[foo]
b = "${bar}"
```

```console
$ knf base.toml override.toml -i
...
[foo.b]
a = 4
d = 1
```

A later config changing `bar.d` changes `foo.b.d` too. `--shallow 'foo.b'`
replaces instead. A value that is replaced is never resolved, but a reference
whose type decides a merge must resolve. To inherit and override, reference
the base in one config (`bar = "${foo}"`) and write the overrides in a later one.

**`--with FILE`**: use one context file for interpolation without merging it
into output; requires `-i`. Each complete reference path prefers the merged
document, then context, including references inside context. Only referenced
context values resolve. A selected container keeps its own children.

```bash
knf foo.toml bar.toml -i --with config.toml
```

Context uses the same format as inputs.

**`--to json|toml`** (`yaml` with the `yaml` feature): convert the final document after merging and
interpolation. Required when there are no input or context files. YAML is
output-only and uses block style.

```bash
knf base.toml prod.toml --to yaml
knf base.json prod.json --to toml --strict
```

TOML dates, times and datetimes become strings in TOML notation. For JSON
output, non-finite TOML floats become `"nan"`, `"inf"` or `"-inf"`; YAML keeps
them as `.nan`, `.inf` and `-.inf`. For TOML output, JSON null object fields are omitted and integers above the
TOML signed range become floats. JSON null array entries are errors.
A lossy conversion warns on stderr. Add **`--strict`** to reject it.

**`--compact`**: disable pretty-printing. Incompatible with `--to yaml`.

**`--exec -- COMMAND [ARGS...]`**: run a command with the merged configuration.
The child inherits standard streams and returns its exit status. Its
`KNF_CONFIG_PATH` environment variable points to a private temporary JSON or
TOML file that is removed when the command finishes; `--to` selects its format.
Cannot be combined with `--dry-run` or `--to yaml`.

```bash
knf base.toml pilot.toml -c seed=42 --exec -- python fit.py --replicate 2
```

## Python API

```python
from knf import deep_merge, dump, dumps, load

config = load(["base.toml", "prod.toml"])
config = load(["base.toml", "prod.toml"], interpolate=True, shallow="db.*")
config = load(["foo.toml", "bar.toml"], interpolate=True, context="config.toml")
pilot = deep_merge(config, {"solver": {"steps": 10}})

text = dumps(config, format="toml")
dump(config, "merged.toml")
```

`load` returns a `dict` and follows the same rules as the CLI. Invalid input
raises `knf.ParseError` or `knf.InterpolationError` (both `ValueError`s); I/O
failures raise the usual `OSError` subclasses. TOML datetimes become `datetime`
objects.

Scripts launched with `--exec` can call `knf.config()` for the resolved
configuration, read fresh on each call. It raises `RuntimeError` outside `--exec`.

```python
import knf

seed = knf.config()["seed"]
```

`dumps` returns JSON, TOML, or YAML text. `dump` writes to a UTF-8 path or text
stream. It infers the format from `.json`, `.toml`, `.yaml`, or `.yml` and
requires `format` for a stream. JSON and TOML support `pretty=False` for compact
output. YAML also accepts `nan` and `inf` and requires `pretty=True`.

`deep_merge(base, exp, overrides)` applies the same pipeline to dicts, left to
right. `deep_merge()` returns `{}`. `deep_merge(cfg, overrides)` is the
two-dict form and does not re-run interpolation unless `interpolate=True`.
`context` is one dict and requires `interpolate=True`. `format` (`"json"` by
default, or `"toml"`) parses whole-string `${env:NAME}` values the way files
of that format do. The result is a new tree.

Don't pass an already-interpolated config with `interpolate=True`: escaped or
environment-supplied `${...}` text would be resolved again.

```python
cfg = load(["base.toml"], interpolate=True)
pilot = deep_merge(cfg, {"solver": {"steps": 10}})

base = load(["base.toml"])  # references stay unresolved
pilot = deep_merge(base, {"port": "${env:PORT}"}, interpolate=True, format="toml")
```

## License

MIT — see [LICENSE](LICENSE).
