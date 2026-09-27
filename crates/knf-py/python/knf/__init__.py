"""Load and merge layered JSON and TOML configuration files into a dict.

The same Rust pipeline the ``knf`` command line runs, called natively::

    >>> from knf import load
    >>> config = load(["base.toml", "prod.json"])
    >>> config["server"]["port"] = 8080

``accumulate`` discovers ordered input paths; ``filter_paths`` selects from an
existing list without filesystem access. Both return ``pathlib.Path`` objects
that can be passed directly to ``load``.
"""

from ._knf import InterpolationError, ParseError, accumulate, filter_paths, load

__all__ = ["InterpolationError", "ParseError", "accumulate", "filter_paths", "load"]
