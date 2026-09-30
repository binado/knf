# knf

Merge layered JSON or TOML config files. No query language, no templates.

## Installation

### Python

```bash
pip install pyknf
```

Installs the `knf` module and the `knf` executable. Wheels for Linux, macOS and
Windows (x86-64 and ARM64); CPython 3.9+.

### Rust

```bash
cargo install knf-cli  # the executable
cargo add knf-core     # the library, imported as `knf`
```

## Usage

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json -c server.port=8080
cat base.json | knf - prod.json -f json
```

Layers merge left to right; the output uses the input format.

| Case | Behaviour |
| --- | --- |
| object ⊕ object | recurse per key |
| array ⊕ anything | replace wholesale |
| scalar ⊕ anything | last wins |
| anything ⊕ null | null overwrites (it is not a delete) |

## CLI

**`-g/--glob`, `-G/--glob-filename`**: keep only inputs whose full path (or
filename) matches the pattern.

```bash
knf config/**/*.toml -g 'config/**/prod.toml'
knf config/**/*.toml -G '*.prod.toml'
```

**`-a/--accumulate`**: collect same-format files in each directory along a
relative target path, shallowest first, with the target last. **`-l/--list-files`**
prints the resulting list without reading files.

```text
foo/conf1.toml
foo/conf2.toml
foo/bar/conf4.toml
```

```bash
knf -a foo/bar/conf4.toml               # = knf foo/conf1.toml foo/conf2.toml foo/bar/conf4.toml
knf -a foo/bar/conf4.toml -G '{conf1,conf4}.toml' --list-files
```

**`-c KEY.PATH=VALUE`**: inline layer applied after all files. The value is
parsed in the selected format and falls back to a string.

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

A later layer changing `bar.d` changes `foo.b.d` too. `--shallow 'foo.b'`
replaces instead. A value that is replaced is never resolved, but a reference
whose type decides a merge must resolve.

**`--with FILE`**: use one context file for interpolation without merging it
into output; requires `-i`. Each complete reference path prefers the merged
document, then context, including references inside context. Only referenced
context values resolve. A selected container keeps its own children.

```bash
knf foo.toml bar.toml -i --with config.toml
generate-config | knf foo.toml bar.toml -i --with - -f toml
```

Context uses the same format as inputs, including `-f`. `--with -` reads stdin
and requires `-f`; stdin cannot also supply a merge layer.
Filtering, accumulation and `--list-files` apply only to merge inputs.

**`-m/--merge-key KEY`**: use one whole-string object reference as defaults for
its parent; requires `-i`. Local fields win and the directive is removed.

```toml
[foo]
a = 1
b = 2
c = 3

[bar]
extends = "${foo}"
c = 4
```

```bash
knf config.toml -i -m extends # bar becomes {a=1, b=2, c=4}
```

The last layer's directive wins; honors `--shallow` at destination key paths.
Use it to inherit and override in one file.
Inherited fields can be referenced as `${bar.a}`; references remain absolute.
Context bases and chained inheritance work. Environment bases must parse as
objects/tables, and their contents remain terminal. Without `-m`, `KEY` is
ordinary data.

**`-f/--format json|toml`**: set the format for parsing, `-c` typing and output.
Required for stdin; never converts between formats. Mixed input formats are an
error.

**`--compact`**: disable pretty-printing.

## Python API

```python
from knf import accumulate, filter_paths, load

config = load(["base.toml", "prod.toml"])
config = load(["base.toml", "prod.toml"], interpolate=True, shallow="db.*")
config = load(["foo.toml", "bar.toml"], interpolate=True, context="config.toml")
config = load(["config.toml"], interpolate=True, merge_key="extends")

files = accumulate("services/api/prod.toml", base_dir=project_root)
files = filter_paths(files, "{defaults,prod}.toml", filename_only=True)
config = load(files)
```

`load` returns a `dict` and follows the same rules as the CLI. Invalid input
raises `knf.ParseError` or `knf.InterpolationError` (both `ValueError`s); I/O
failures raise the usual `OSError` subclasses. TOML datetimes become `datetime`
objects.

## License

MIT — see [LICENSE](LICENSE).
