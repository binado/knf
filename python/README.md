# pyknf

Merge layered JSON or TOML config files into a `dict`, using the same Rust
engine as the [`knf`](https://github.com/binado/knf) CLI.

```bash
pip install pyknf  # the `knf` module and the `knf` executable
```

```python
from knf import deep_merge, dump, dumps, filter_paths, load, merge

config = load(["base.toml", "prod.toml"])
config = load(["base.toml", "prod.toml"], interpolate=True, shallow="db.*")
config = load(["foo.toml", "bar.toml"], interpolate=True, context="config.toml")
pilot = merge(config, {"solver": {"steps": 10}}, interpolate=True, shallow="solver")
pilot = deep_merge(config, {"solver": {"steps": 10}})

files = filter_paths(candidates, "{defaults,prod}.toml", filename_only=True)
config = load(files)

text = dumps(config, format="toml")
dump(config, "merged.toml")
```

- `load` merges left to right: objects recurse; arrays, scalars and `None`
  replace. All files must share one format. An empty list returns `{}`.
- `merge(base, exp, overrides)` applies that pipeline to dicts, left to
  right. `merge()` returns `{}`. `context` is one dict and requires
  `interpolate=True`. `format` (`"json"` by default, or `"toml"`) parses
  whole-string `${env:NAME}` values the way files of that format do. The
  result is a new tree.
- `deep_merge(cfg, overrides)` is `merge(cfg, overrides)`. It does not
  re-run interpolation.
- `interpolate=True` resolves `${key.path}` and `${env:NAME}` against the
  final document. A whole-string reference merges as the value it names, so
  an object referent merges with an object from another file.
- `context="config.toml"` supplies one same-format filepath without merging it;
  requires `interpolate=True`. Complete paths prefer output, then context,
  including context dependencies. Only referenced context values resolve.
- `shallow="PATTERN"` replaces matching key paths wholesale (`"*"`, `"db"`,
  `"db.*"`).
- `filter_paths` filters a list of paths by glob without I/O.
- `dumps` returns JSON, TOML, or YAML text. `dump` infers the format from
  `.json`, `.toml`, `.yaml`, or `.yml` and requires an explicit format for text
  streams. YAML accepts the JSON types plus `nan` and `inf`, and requires
  `pretty=True`. See `dumps_toml` docs for TOML limits.
- Errors: `knf.ParseError` and `knf.InterpolationError` (both `ValueError`s),
  `ValueError` for bad arguments, and `OSError` subclasses for I/O failures.
- TOML datetimes become `datetime`, `date` or `time` objects.

Scripts launched with `knf --exec` can read the resolved dict from `knf.config()` (`RuntimeError` outside `--exec`).

See the [project README](https://github.com/binado/knf) for details.
