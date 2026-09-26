"""Load and merge layered JSON and TOML configuration files into a dict.

The same Rust pipeline the ``knf`` command line runs, called natively::

    >>> from knf import load
    >>> config = load(["base.toml", "prod.json"])
    >>> config["server"]["port"] = 8080
"""

from ._knf import InterpolationError, ParseError, load

__all__ = ["InterpolationError", "ParseError", "load"]
