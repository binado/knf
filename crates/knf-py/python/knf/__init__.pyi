import os
from collections.abc import Mapping, Sequence
from typing import Any, Union

class KnfError(Exception):
    """A file could not be read, parsed or merged."""

def deep_merge(
    files: Sequence[Union[str, os.PathLike[str]]],
    *,
    override: Union[Mapping[str, Any], None] = None,
) -> dict[str, Any]:
    """Merge layered JSON and TOML files into one ``dict``.

    ``files`` are merged left to right; ``override``, if given, is merged last,
    as one more layer -- the Python spelling of ``knf --set``. Objects merge key
    by key; arrays, scalars and ``None`` replace wholesale.
    """
