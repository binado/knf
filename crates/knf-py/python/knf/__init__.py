"""Merge layered JSON and TOML configuration files into a dict.

The same Rust pipeline the ``knf`` command line runs, called natively::

    >>> from knf import deep_merge
    >>> deep_merge(["base.toml", "prod.json"], override={"server": {"port": 8080}})
"""

from ._knf import KnfError, deep_merge

__all__ = ["KnfError", "deep_merge"]
