import os
from collections.abc import Sequence
from typing import Any, Union

class ParseError(ValueError):
    """A file is not valid JSON or TOML, or is not an object at the top level."""

class InterpolationError(ValueError):
    """A configuration reference cannot be resolved."""

def load(
    files: Sequence[Union[str, os.PathLike[str]]],
    *,
    interpolate: bool = False,
) -> dict[str, Any]:
    """Load and merge layered JSON and TOML files into one ``dict``.

    ``files`` are merged left to right. Objects merge key by key; arrays,
    scalars and ``None`` replace wholesale.
    With ``interpolate=True``, references resolve once against the final merged
    document; ``${env:NAME}`` reads the process environment.

    Raises:
        FileNotFoundError, PermissionError, IsADirectoryError: As ``open()``
            would, with ``.filename`` set.
        ParseError: A file is not a valid JSON or TOML document.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: A path has no ``.json``/``.toml`` extension.
    """
