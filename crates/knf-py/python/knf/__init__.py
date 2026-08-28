"""Merge layered JSON and TOML configuration files into a dict.

The pipeline is the same Rust one the ``knf`` command line runs, so a document
merged here and a document merged by ``knf base.toml prod.toml`` are the same
document.  Nothing is spawned: ``deep_merge`` is a native call.

    >>> from knf import deep_merge
    >>> deep_merge(["base.toml", "prod.json"], rules={"plugins": "append"})

Install the command line separately -- it is a different distribution::

    pip install knf-config   # this library, `from knf import deep_merge`
    pip install knf-cli      # the `knf` executable
"""

from ._knf import (
    InterpolationError,
    KnfError,
    LoadError,
    MergeError,
    ParseError,
    PathError,
    RuleError,
    deep_merge,
)

__all__ = [
    "InterpolationError",
    "KnfError",
    "LoadError",
    "MergeError",
    "ParseError",
    "PathError",
    "RuleError",
    "deep_merge",
]
