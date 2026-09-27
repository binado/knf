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

files = accumulate("services/api/prod.toml", base_dir=project_root)
files = filter_paths(files, "{defaults,prod}.toml", filename_only=True)
config = load(files)
```

- `load` merges left to right: objects recurse; arrays, scalars and `None`
  replace. All files must share one format. An empty list returns `{}`.
- `interpolate=True` resolves `${key.path}` and `${env:NAME}` after merging.
- `shallow="PATTERN"` replaces matching key paths wholesale (`"*"`, `"db"`,
  `"db.*"`).
- `accumulate` collects same-format files along a relative target path, with
  the target last. `filter_paths` filters a list by glob without I/O.
- Errors: `knf.ParseError` and `knf.InterpolationError` (both `ValueError`s),
  `ValueError` for bad arguments, and `OSError` subclasses for I/O failures.
- TOML datetimes become `datetime`, `date` or `time` objects.

See the [project README](https://github.com/binado/knf) for details.
