import os
from collections.abc import Sequence
from typing import Any, Union

class ParseError(ValueError):
    """A file is not valid JSON or TOML, or is not an object at the top level."""

class InterpolationError(ValueError):
    """A configuration reference cannot be resolved."""

def deep_merge(
    files: Sequence[Union[str, os.PathLike[str]]],
    *,
    override: Union[dict[str, Any], None] = None,
    interpolate: bool = False,
) -> dict[str, Any]:
    """Merge layered JSON and TOML files into one ``dict``.

    ``files`` are merged left to right; ``override``, if given, is merged last,
    as one more layer -- the Python spelling of ``knf --set``. Objects merge key
    by key; arrays, scalars and ``None`` replace wholesale.
    With ``interpolate=True``, references resolve once against the final merged
    document; ``${env:NAME}`` reads the process environment.

    Raises:
        FileNotFoundError, PermissionError, IsADirectoryError: As ``open()``
            would, with ``.filename`` set.
        ParseError: A file is not a valid JSON or TOML document.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: A path has no ``.json``/``.toml`` extension, or an
            ``override`` value does not fit the document model.
        TypeError: ``override`` is not a ``dict`` of JSON-like values.
    """
