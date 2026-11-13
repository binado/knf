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
that can be passed directly to ``load``.
"""

from ._knf import InterpolationError, ParseError, accumulate, dumps_toml, filter_paths, load


def dump_toml(data, fp, *, pretty=True):
    """Write a dict as TOML to a text stream, using :func:`dumps_toml`.

    Open files with ``encoding="utf-8"``. Serialization completes before writing;
    stream errors propagate unchanged. Returns None.
    """
    fp.write(dumps_toml(data, pretty=pretty))


__all__ = [
    "InterpolationError", "ParseError", "accumulate", "dump_toml", "dumps_toml",
    "filter_paths", "load",
]
