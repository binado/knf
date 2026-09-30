# pyknf

Merge layered JSON or TOML config files into a `dict`, using the same Rust
engine as the [`knf`](https://github.com/binado/knf) CLI.

```bash
pip install pyknf  # the `knf` module and the `knf` executable
```

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

- `load` merges left to right: objects recurse; arrays, scalars and `None`
  replace. All files must share one format. An empty list returns `{}`.
- `interpolate=True` resolves `${key.path}` and `${env:NAME}` against the
  final document. A whole-string reference merges as the value it names, so
  an object referent merges with an object from another file.
- `context="config.toml"` supplies one same-format filepath without merging it;
  requires `interpolate=True`. Complete paths prefer output, then context,
  including context dependencies. Only referenced context values resolve.
- `shallow="PATTERN"` replaces matching key paths wholesale (`"*"`, `"db"`,
  `"db.*"`).
- `merge_key="extends"` uses the object referenced by `extends = "${foo}"`
  as defaults for its parent. Local fields win; the directive is removed.
  Requires `interpolate=True` and honors `shallow` at destination paths.
- `accumulate` collects same-format files along a relative target path, with
  the target last. `filter_paths` filters a list by glob without I/O.
- Errors: `knf.ParseError` and `knf.InterpolationError` (both `ValueError`s),
  `ValueError` for bad arguments, and `OSError` subclasses for I/O failures.
- TOML datetimes become `datetime`, `date` or `time` objects.

See the [project README](https://github.com/binado/knf) for details.
