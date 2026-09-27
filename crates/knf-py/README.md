# pyknf

Load and merge layered JSON and TOML configuration files into a `dict`. This is the same
Rust merge the [`knf`](https://github.com/binado/knf) command line runs, called
natively.

```bash
pip install pyknf    # the module, and the `knf` executable
```

```python
from knf import load

config = load(["base.toml", "prod.toml"])
config["server"]["port"] = 8080
```

Discover and filter paths before loading them:

```python
from knf import accumulate, filter_paths, load

files = accumulate("services/api/prod.toml", base_dir=project_root)
files = filter_paths(files, "{defaults,prod}.toml", filename_only=True)
config = load(files, interpolate=True)
```

Both helpers accept strings and `os.PathLike` objects and return `pathlib.Path`
objects. `accumulate` discovers same-format files along the relative target's
directories, excluding files directly in the base directory, sorting each
directory by filename and placing the target last. It does not parse contents.
Without `base_dir`, results are relative to the working directory; an explicit
base produces absolute paths without changing the working directory or
canonicalizing symlinks. Absolute targets, `..`, stdin and unsupported
extensions are rejected.

`filter_paths` filters existing candidates without filesystem access and
preserves order and duplicates. Patterns match entire paths unless
`filename_only=True`; matching uses the command line's case-sensitive glob
syntax. Matching precedes conversion to `Path`, which normalizes components
such as `./`. Invalid targets/patterns and other non-regular targets raise
`ValueError`; discovery I/O failures raise `OSError` subclasses with `.errno`
and `.filename`, including `IsADirectoryError` for directory targets.

Pass `interpolate=True` to resolve references after all files have been merged:

```python
config = load(["base.toml", "prod.toml"], interpolate=True)
```

`${key.path}` reads a value from the final config, including nested keys and
array elements such as `${servers[0].host}`. A reference that occupies the
whole string keeps the value's type: `"${server.port}"` can become an `int`,
and `"${server}"` can become a `dict`. Within other text, it becomes a string:
`"http://${server.host}:${server.port}/"`. `${env:NAME}` reads the process
environment; a whole-string environment reference parses a native JSON or TOML value with string
fallback, matching the CLI's inline typing, while an embedded one inserts raw text. Use `$$` for a literal
`$`. Interpolation is off by default, leaving reference strings untouched.
Invalid or missing references and cycles raise `knf.InterpolationError`, a
`ValueError`, with the affected key paths.

Files are merged left to right, exactly like `knf base.toml prod.toml`.
Objects merge key by key. Arrays, scalars and `None` replace wholesale. Make
additional changes to the returned dict in Python; for example,
`config["server"]["port"] = 8080` updates a nested setting.

Pass `shallow="PATTERN"` to replace matching full key paths wholesale with the
CLI's key-path glob syntax. `shallow="*"` replaces top-level values;
`shallow="db"` replaces all of `db`, while `shallow="db.*"` replaces its
immediate children. Dots separate keys; single-quoted spans are literal, as in
`shallow="'foo.bar'.*"`. Matching ancestors stop traversal. The default,
`shallow=None`, keeps deep merging. Empty or invalid patterns raise `ValueError`
before files are read. Interpolation runs once after merging.

```python
config = load(["base.toml", "prod.toml"], shallow="{db,cache}.*")
```

A file that can't be read raises `FileNotFoundError`, `PermissionError` or
`IsADirectoryError`, as `open()` would. Invalid JSON or TOML raises
`knf.ParseError`, a `ValueError` like `json.JSONDecodeError`. TOML datetimes
return native `datetime.datetime`, `datetime.date`, or `datetime.time` objects,
matching Python's `tomllib` type mapping. Offset datetimes retain fixed UTC
offsets; local datetimes and times have no timezone. Fractional seconds truncate
to microseconds. A date/time Python cannot represent raises `ValueError` naming
its key path.

All inputs must use one format. Mixed JSON/TOML extensions raise `ValueError`
before document contents are read. An empty file list returns `{}`. JSON nulls
and large unsigned integers, and TOML non-finite floats, retain their values.

This changes the previous datetime-string and mixed-input behavior. The CLI
also now uses `-f/--format` for parsing, inline typing and output together;
`--input-format` and `--null-as` have been removed. No JSON/TOML conversion is
performed. Parsed values are preserved; source whitespace and comments are not.

See the [project README](https://github.com/binado/knf#merging) for the merge
semantics in full.
