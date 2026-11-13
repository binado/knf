import os
from pathlib import Path
from collections.abc import Sequence
from typing import Any, Optional, TextIO, Union

def dumps_toml(data: dict[str, Any], *, pretty: bool = True) -> str:
    """Serialize a dict as TOML with a trailing newline.

    Supports str, bool, signed 64-bit int, float, list, tuple, dict with string
    keys, and datetime/date/time. None is unsupported. Datetime offsets must be
    whole minutes; times must be naive. Preserves dict insertion order.
    Raises TypeError for unsupported types and ValueError for invalid values,
    cycles, or nesting deeper than 128 containers, with key paths.
    """

def dump_toml(data: dict[str, Any], fp: TextIO, *, pretty: bool = True) -> None:
    """Serialize with dumps_toml, then write to a text stream (UTF-8 for files)."""

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
    context: Optional[Union[str, os.PathLike[str]]] = None,
    merge_key: Optional[str] = None,
) -> dict[str, Any]:
    """Load and merge homogeneous JSON or TOML files into one ``dict``.

    ``files`` merge left to right. ``shallow`` is a key-path glob whose matches
    replace wholesale (``"*"``, ``"foo"``, ``"foo.*"``). ``interpolate=True``
    resolves ``${key.path}`` and ``${env:NAME}`` against the final document;
    a whole-string reference merges exactly as the value it names.
    ``context`` is one filepath used only for interpolation; requires
    ``interpolate=True``. Complete paths prefer the merged document, then
    context, including references inside context. Unused context references
    are not resolved. All files must share one format.
    ``merge_key`` selects a literal key containing one whole-string object
    reference. Its fields supply defaults; local fields win and the directive
    is removed. Requires ``interpolate=True`` and honors ``shallow``.

    Raises:
        FileNotFoundError, PermissionError, IsADirectoryError: As ``open()``
            would, with ``.filename`` set.
        ParseError: A file is not a valid JSON or TOML document.
        TypeError: Context is not a filepath or merge_key is not a string.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: Invalid glob, unknown extension, mixed formats, invalid
            context or merge_key arguments, or an unrepresentable datetime.
    """
