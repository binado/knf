import os
from pathlib import Path
from collections.abc import Sequence
from typing import Any, Literal, Optional, TextIO, Union

def dumps(
    data: dict[str, Any],
    *,
    format: Literal["json", "toml", "yaml"],
    pretty: bool = True,
) -> str:
    """Serialize a dict as JSON, TOML, or YAML with a trailing newline.

    JSON supports None, bool, str, finite floats, integers from -2**63 through
    2**64 - 1, lists, tuples, and dicts with string keys. YAML also supports
    non-finite floats and requires pretty=True. For TOML, see dumps_toml. Types
    the format lacks raise TypeError, and values it cannot hold raise
    ValueError, with the key path.
    """

def dump(
    data: dict[str, Any],
    target: Union[str, os.PathLike[str], TextIO],
    *,
    format: Optional[Literal["json", "toml", "yaml"]] = None,
    pretty: bool = True,
) -> None:
    """Serialize with dumps before writing to a UTF-8 path or text stream.

    Infer the format from a path's .json, .toml, .yaml, or .yml extension.
    Text streams require an explicit format. I/O errors propagate unchanged.
    """

def config() -> dict[str, Any]:
    """Return the resolved configuration of the `knf --exec` run.

    Read from KNF_CONFIG_PATH on every call, so each caller gets its own dict.
    Raises RuntimeError outside --exec or if the snapshot cannot be loaded.
    """

def deep_merge(cfg: dict[str, Any], overrides: dict[str, Any]) -> dict[str, Any]:
    """Return a new dict by merging dict pairs and replacing other values.

    Values outside a recursive merge are shared with the inputs, not copied.
    """

def dumps_toml(data: dict[str, Any], *, pretty: bool = True) -> str:
    """Serialize a dict as TOML with a trailing newline.

    Supports str, bool, signed 64-bit int, float, list, tuple, dict with string
    keys, and datetime/date/time. None is unsupported. Datetime offsets must be
    whole minutes; times must have no tzinfo. Keys keep insertion order, except
    that TOML writes tables after scalar keys.

    Raises TypeError for unsupported types and ValueError for invalid values,
    cycles, or nesting beyond 128 levels. Errors for such values name the key
    path.
    """

def dump_toml(data: dict[str, Any], fp: TextIO, *, pretty: bool = True) -> None:
    """Serialize with dumps_toml, then write to a text stream.

    Serialization completes first; stream errors propagate unchanged. Open files
    with ``encoding="utf-8"``.
    """

class ParseError(ValueError):
    """A file is not valid JSON or TOML, or is not an object at the top level."""

class InterpolationError(ValueError):
    """A configuration reference cannot be resolved."""

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

    Raises:
        FileNotFoundError, PermissionError, IsADirectoryError: As ``open()``
            would, with ``.filename`` set.
        ParseError: A file is not a valid JSON or TOML document.
        TypeError: Context is not a filepath.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: Invalid glob, unknown extension, mixed formats, invalid
            context arguments, or an unrepresentable datetime.
    """
