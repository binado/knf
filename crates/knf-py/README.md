# knf-config

Merge layered JSON and TOML configuration files into a `dict`.

```console
pip install knf-config
```

```python
from knf import deep_merge

config = deep_merge(["base.toml", "prod.json"], rules={"plugins": "append"})
```

Files are **layers**, merged left to right. JSON and TOML mix freely — both
parse into one intermediate representation, so a JSON layer over a TOML layer
needs no conversion in between. The merge is the same Rust pipeline the `knf`
command line runs, called directly: no subprocess, no re-serialisation.

> The command line is a **separate distribution**: `pip install knf-cli` puts the
> `knf` executable on your `PATH`. This one is the library.

## `deep_merge`

```python
deep_merge(
    paths,                  # iterable of str | os.PathLike; "-" reads stdin
    *,
    input_format=None,      # "json" | "toml" — override extension inference
    strict=False,           # error when a layer changes a key's type
    rules=None,             # {"key.path": "append" | "replace" | "fail"}
    overlays=(),            # dicts appended after every file, in order
    interpolate=False,      # resolve ${key.path} and ${env:VAR}
    env=None,               # what ${env:VAR} reads; None is os.environ
) -> dict
```

Objects merge key by key. Arrays, scalars and `None` all replace wholesale —
`None` is an ordinary value that overwrites, not a delete instruction. With one
argument and no options, `deep_merge` is the identity:
`deep_merge([f]) == json.load(open(f))`.

`paths` is an iterable, and a bare string is rejected rather than read one
character at a time. `"-"` reads standard input and therefore needs an explicit
`input_format`, since stdin has no extension.

### `rules=`

One mapping from key path to strategy. Every strategy is *terminal* — it takes
the whole value at its path and does not recurse — so no rule may sit beneath
another, and one that does raises `RuleError` before any file is opened.

| strategy | at that path |
| --- | --- |
| `"append"` | concatenate the arrays; both sides must be arrays |
| `"replace"` | take the later layer's value whole, even object over object |
| `"fail"` | error if a later layer sets it again; the first layer to define it pins it |

```python
deep_merge(["base.toml", "prod.toml"], rules={"plugins": "append", "db": "replace"})
```

A key path is dotted (`db.host`), so a key containing a literal dot is not
addressable this way — put it in a file, or in an `overlays=` dict. An index like
`servers[0]` may be *read* by a `${...}` reference but never written, so it is
not a legal rule path either.

### `overlays=`

Dicts, appended after every file and merged as ordinary terminal layers. This is
the `--set` of the command line, spelled the way Python already spells nested
data:

```python
deep_merge(["base.toml"], overlays=[{"server": {"port": 8080}}])
```

### `interpolate=`

Off by default, and deliberately: knf sits upstream of tools whose own syntax is
`${...}` — compose files, Actions workflows, Helm charts — so eating those
without being asked would be silent corruption. On, one pass runs over the
*merged* document:

```python
deep_merge(["app.toml"], interpolate=True, env={"PORT": "8080"})
```

```toml
root     = "/srv"
data_dir = "${root}/data"    # -> "/srv/data"
port     = "${env:PORT}"     # -> 8080, an int, not "8080"
url      = "x:${env:PORT}"   # -> "x:8080"
literal  = "$${NOT_A_REF}"   # -> "${NOT_A_REF}"
```

A reference that is the **whole** string takes the referent's value *and type*,
so it can yield a number, a list or a dict. Embedded in text it stringifies, and
a container has no format-independent spelling there, so that is an error rather
than a guess. Document references resolve transitively; environment values are
terminal and never re-scanned. Cycles are an error.

`env=` is snapshotted before the merge runs, so passing one makes the result a
function of the arguments alone; `os.environ` is never consulted when it is
given. `${env:NAME}` types its value exactly as the CLI's `--set` does: `8080` is
an int, `foo` is a string.

## Types

| in the document | in Python |
| --- | --- |
| `null` | `None` |
| `true` / `false` | `bool` |
| integer | `int` (arbitrary precision — a value past `2**63` survives) |
| float | `float` (`inf` and `nan` included) |
| string | `str` |
| TOML offset date-time | aware `datetime.datetime` |
| TOML local date-time | naive `datetime.datetime` |
| TOML local date | `datetime.date` |
| TOML local time | `datetime.time` |
| array | `list` |
| table / object | `dict`, in the document's key order |

**Python is the widest target knf emits to.** `knf -f toml` refuses nulls and
integers past `2**63 - 1`; `knf -f json` refuses `inf` and `nan`. Python spells
all of them, so `deep_merge` refuses none — there is no emit step to refuse at.

The rejection lives on the way *in* instead. An `overlays=` dict takes `None`,
`bool`, `int`, `float`, `str`, `list`, `tuple` and `dict`. An `int` outside
`[-2**63, 2**64)` raises `OverflowError` naming its path rather than being
truncated; a non-`str` key or any other type raises `TypeError`, `datetime`
included — a datetime carries a TOML source spelling, and one is only ever read,
never synthesised.

## Exceptions

```
knf.KnfError
├── LoadError            .path        a path that could not become a layer
├── ParseError                        a layer that is not valid JSON or TOML
├── MergeError           .path        the fold rejected a layer
├── RuleError            .paths       the rule set is not legal
├── InterpolationError   .problems    a ${...} reference did not resolve
└── PathError                         a malformed dotted key in rules=
```

Each carries the structured half of its message as an attribute, so nothing has
to be recovered by parsing a string: `.path` is a `str` for `LoadError` and a
tuple of keys for `MergeError`, `.paths` is one such tuple per offending rule,
and `.problems` is a tuple of `(kind, path, detail)` triples.

Failures in the *arguments* rather than the documents raise the builtins you
would expect: `TypeError`, `ValueError`, `OverflowError`, and `FileNotFoundError`
for a path that is not there.

## License

MIT
