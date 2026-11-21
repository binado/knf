"""Load and merge layered JSON and TOML configuration files into a dict.

Inputs must share one format. TOML dates and times become Python date/time
objects, with fractional seconds truncated to microseconds. Pass
``shallow="db.*"`` to ``load`` to replace the immediate children of ``db``
wholesale, using the same key-path glob syntax as the command line.

The same Rust pipeline the ``knf`` command line runs, called natively::

    >>> from knf import load
    >>> config = load(["base.toml", "prod.toml"])
    >>> config["server"]["port"] = 8080

``filter_paths`` selects from a list of paths without filesystem access and
returns ``pathlib.Path`` objects that can be passed directly to ``load``. ``deep_merge`` applies in-memory
overrides. ``dumps`` and ``dump`` serialize a dict as JSON, TOML, or YAML. Under
``knf --exec``, ``config()`` returns the resolved configuration.
"""

import os as _os
from typing import Any

from ._knf import (
    InterpolationError,
    ParseError,
    dumps,
    dumps_toml,
    filter_paths,
    load,
)


def dump(data, target, *, format=None, pretty=True):
    """Write a dict to a UTF-8 path or text stream after serialization succeeds.

    If omitted, the format is inferred from a path's .json, .toml, .yaml,
    or .yml extension. Text streams require an explicit format.
    """
    if isinstance(target, (str, _os.PathLike)):
        path = _os.fspath(target)
        if format is None:
            suffix = _os.path.splitext(path)[1].lower()
            if suffix not in (".json", ".toml", ".yaml", ".yml"):
                raise ValueError("cannot infer format from path; pass format explicitly")
            format = "yaml" if suffix == ".yml" else suffix[1:]
        text = dumps(data, format=format, pretty=pretty)
        with open(path, "w", encoding="utf-8") as fp:
            fp.write(text)
    else:
        if format is None:
            raise ValueError("format is required for a text stream")
        target.write(dumps(data, format=format, pretty=pretty))


def deep_merge(cfg, overrides):
    """Return a new dict with overrides applied recursively to dict pairs.

    Other values replace wholesale, including lists and None. Values outside a
    recursive merge are shared with the inputs. Interpolation happens in load().
    """
    merged = cfg.copy()
    for key, value in overrides.items():
        original = cfg.get(key)
        if isinstance(original, dict) and isinstance(value, dict):
            value = deep_merge(original, value)
        merged[key] = value
    return merged


def dump_toml(data, fp, *, pretty=True):
    """Serialize with :func:`dumps_toml`, then write to the text stream ``fp``."""
    fp.write(dumps_toml(data, pretty=pretty))


def config() -> dict[str, Any]:
    """Return the resolved configuration of the ``knf --exec`` run.

    The snapshot named by ``KNF_CONFIG_PATH`` is read on every call, so each
    caller gets an independent dict.

    Returns
    -------
    dict[str, Any]
        The merged, interpolated configuration.

    Raises
    ------
    RuntimeError
        If not launched by ``knf --exec``, or the snapshot cannot be loaded.
    """
    path = _os.environ.get("KNF_CONFIG_PATH")
    if not path:
        raise RuntimeError(
            "knf.config() requires a run launched with knf [FILES] --exec -- COMMAND"
        )
    # Snapshot is already resolved; skip interpolation to avoid re-expanding literal ${...}.
    try:
        return load([path], interpolate=False)
    except (OSError, ValueError) as exc:
        raise RuntimeError(f"cannot load KNF_CONFIG_PATH={path!r}: {exc}") from exc


__all__ = [
    "InterpolationError", "ParseError", "config", "deep_merge", "dump",
    "dump_toml", "dumps", "dumps_toml", "filter_paths", "load",
]
