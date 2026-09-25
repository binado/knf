# pyknf

Merge layered JSON and TOML configuration files into a `dict`. This is the same
Rust merge the [`knf`](https://github.com/binado/knf) command line runs, called
natively.

```bash
pip install pyknf    # the module, and the `knf` executable
```

```python
from knf import deep_merge

config = deep_merge(["base.toml", "prod.json"], override={"server": {"port": 8080}})
```

Pass `interpolate=True` to resolve references after all files and the override
have been merged:

```python
config = deep_merge(["base.toml", "prod.json"], interpolate=True)
```

`${key.path}` reads a value from the final config, including nested keys and
array elements such as `${servers[0].host}`. A reference that occupies the
whole string keeps the value's type: `"${server.port}"` can become an `int`,
and `"${server}"` can become a `dict`. Within other text, it becomes a string:
`"http://${server.host}:${server.port}/"`. `${env:NAME}` reads the process
environment; a whole-string environment reference uses the CLI's JSON-or-string
typing rule, while an embedded one inserts raw text. Use `$$` for a literal
`$`. Interpolation is off by default, leaving reference strings untouched.
Invalid or missing references and cycles raise `knf.InterpolationError`, a
`ValueError`, with the affected key paths.

Files are merged left to right, exactly like `knf base.toml prod.json`.
Objects merge key by key. Arrays, scalars and `None` replace wholesale. If you
pass `override`, it is merged last as one more layer, which makes it the
Python version of `--set`. It must be a `dict` of JSON-like values: `None`,
`bool`, `int` (within 64 bits), `float`, `str`, `list`/`tuple` and nested
`dict`s with `str` keys, at most 128 levels deep and without cycles. Anything
else raises `TypeError` or `ValueError` naming the key path, before any file
is read.

A file that can't be read raises `FileNotFoundError`, `PermissionError` or
`IsADirectoryError`, as `open()` would. Invalid JSON or TOML raises
`knf.ParseError`, a `ValueError` like `json.JSONDecodeError`. TOML datetimes
come back as their TOML spelling in a `str`.

See the [project README](https://github.com/binado/knf#merging) for the merge
semantics in full.
