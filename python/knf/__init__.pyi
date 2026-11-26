import os
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
    non-finite floats and requires pretty=True. TOML supports str, bool,
    signed 64-bit int, float, list, tuple, dict with string keys, and
    datetime/date/time. Datetime offsets must be whole minutes and times must
    have no tzinfo. TOML writes tables after scalar keys and otherwise
    preserves insertion order.

    Unsupported types raise TypeError. Invalid values, cycles, and nesting
    beyond 128 levels raise ValueError. Errors for such values name the key
    path.
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

def deep_merge(
    *configs: dict[str, Any],
    interpolate: bool = False,
    shallow: Optional[str] = None,
    context: Optional[dict[str, Any]] = None,
    format: Literal["json", "toml"] = "json",
) -> dict[str, Any]:
    """Merge in-memory dicts with the same pipeline as ``load``.

    ``configs`` merge left to right. ``deep_merge()`` returns ``{}``.
    ``deep_merge(cfg, overrides)`` is the two-dict form. ``shallow`` is a
    key-path glob whose matches replace wholesale (``"*"``, ``"foo"``,
    ``"foo.*"``). ``interpolate=True`` resolves ``${key.path}`` and
    ``${env:NAME}`` against the final document; a whole-string reference
    merges as the value it names. ``context`` is one dict used only for
    interpolation and requires ``interpolate=True``. Complete paths prefer
    the merged document, then context, including references inside context.
    Unused context references are not resolved. ``format`` selects how
    whole-string environment values are parsed (``"json"`` by default, or
    ``"toml"``), matching ``load`` of files in that format.

    Each config, and ``context``, must be a dict. Values inside them may be
    ``None``, bool, str, float, integers from ``-2**63`` through
    ``2**64 - 1``, list, tuple, dict with string keys, and datetime, date, or
    time. Tuples become lists. The result is a new tree.

    Don't pass an already-interpolated config with ``interpolate=True``:
    escaped or environment-supplied ``${...}`` text would be resolved again.

    Raises:
        TypeError: A config or context is not a dict, a key is not a string,
            or a value has an unsupported type.
        InterpolationError: A reference is invalid, unresolved, or cyclic.
        ValueError: Invalid glob or format, context without interpolation,
            an integer outside the 64-bit range, a self-referencing container,
            nesting beyond 128 containers, a time with a tzinfo, a datetime
            whose offset isn't whole minutes or whose ``utcoffset()`` is
            ``None``, an otherwise invalid datetime, or a TOML environment
            datetime that Python can't represent.

    Errors converting a value inside an input name the input (``configs[i]``,
    zero-based, or ``context``) and the key path.
    """

class ParseError(ValueError):
    """A file is not valid JSON or TOML, or is not an object at the top level."""

class InterpolationError(ValueError):
    """A configuration reference cannot be resolved."""

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
