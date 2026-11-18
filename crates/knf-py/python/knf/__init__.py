"""Load and merge layered JSON and TOML configuration files into a dict.

Inputs must share one format. TOML dates and times become Python date/time
objects, with fractional seconds truncated to microseconds.
Pass ``shallow="db.*"`` to ``load`` to replace the immediate children of ``db``
wholesale, using the same key-path glob syntax as the command line.

The same Rust pipeline the ``knf`` command line runs, called natively::

    >>> from knf import load
    >>> config = load(["base.toml", "prod.toml"])
    >>> config["server"]["port"] = 8080

``accumulate`` discovers ordered input paths; ``filter_paths`` selects from an
existing list without filesystem access. Both return ``pathlib.Path`` objects
that can be passed directly to ``load``. ``deep_merge`` applies in-memory
overrides to a configuration dict. ``dumps_toml`` and ``dump_toml``
serialize a dict back to TOML.
"""

from ._knf import InterpolationError, ParseError, accumulate, dumps_toml, filter_paths, load


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


__all__ = [
    "InterpolationError", "ParseError", "accumulate", "deep_merge", "dump_toml",
    "dumps_toml", "filter_paths", "load",
]
