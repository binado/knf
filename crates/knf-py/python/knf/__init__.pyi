import os
from pathlib import Path
from collections.abc import Sequence
from typing import Any, Optional, Union

class ParseError(ValueError):
    """A file is not valid JSON or TOML, or is not an object at the top level."""

class InterpolationError(ValueError):
    """A configuration reference cannot be resolved."""

def accumulate(
    target: Union[str, os.PathLike[str]],
    *,
    base_dir: Optional[Union[str, os.PathLike[str]]] = None,
) -> list[Path]:
    """Discover same-format files along a relative target path, with the target last.

    Paths are absolute only when ``base_dir`` is given.

    Raises:
        ValueError: Invalid target or a target that is not a regular file.
        OSError: Discovery failed, with filesystem subclass and filename set.
    """

def filter_paths(
    files: Sequence[Union[str, os.PathLike[str]]],
    pattern: str,
    *,
    filename_only: bool = False,
) -> list[Path]:
    """Filter paths by glob without I/O, preserving order and duplicates.

    Raises ValueError for an invalid pattern.
    """

def load(
    files: Sequence[Union[str, os.PathLike[str]]],
    *,
    interpolate: bool = False,
    shallow: Optional[str] = None,
) -> dict[str, Any]:
    """Load and merge homogeneous JSON or TOML files into one ``dict``.

    ``files`` merge left to right. ``shallow`` is a key-path glob whose matches
    replace wholesale (``"*"``, ``"foo"``, ``"foo.*"``). ``interpolate=True``
    resolves ``${key.path}`` and ``${env:NAME}`` after merging.

    Raises:
        FileNotFoundError, PermissionError, IsADirectoryError: As ``open()``
            would, with ``.filename`` set.
        ParseError: A file is not a valid JSON or TOML document.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: Invalid glob, unknown extension, mixed formats, or an
            unrepresentable datetime.
    """
