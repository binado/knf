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

    Excludes files directly in the base directory. Without a base, returns
    working-directory-relative paths; an explicit base produces absolute paths.
    Does not read configuration contents or canonicalize symlinks.

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
    """Filter existing candidates without I/O, preserving order and duplicates.

    Case-sensitive glob matching uses native bytes against the supplied spelling
    or just its filename. Returned Path objects normalize components such as './'.
    An empty selection is allowed. Invalid patterns raise ValueError.
    """

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
