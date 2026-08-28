import os
from collections.abc import Iterable, Mapping, Sequence
from typing import Any, Literal, TypedDict

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

Format = Literal["json", "toml"]
Strategy = Literal["append", "replace", "fail"]

class MergeOptions(TypedDict, total=False):
    """Every keyword `deep_merge` takes, for reuse across calls.

    There is no second options *type* at runtime -- the keywords are the
    options -- so this exists only to let a checker see through ``**opts``::

        opts: MergeOptions = {"strict": True, "rules": {"plugins": "append"}}
        base = deep_merge(["base.toml"], **opts)
        prod = deep_merge(["base.toml", "prod.toml"], **opts)
    """

    input_format: Format | None
    strict: bool
    rules: Mapping[str, Strategy] | None
    overlays: Iterable[Mapping[str, Any]]
    interpolate: bool
    env: Mapping[str, str] | None

def deep_merge(
    paths: Iterable[str | os.PathLike[str]],
    *,
    input_format: Format | None = None,
    strict: bool = False,
    rules: Mapping[str, Strategy] | None = None,
    overlays: Iterable[Mapping[str, Any]] = (),
    interpolate: bool = False,
    env: Mapping[str, str] | None = None,
) -> dict[str, Any]: ...

class KnfError(Exception): ...

class LoadError(KnfError):
    path: str | None

class ParseError(KnfError): ...

class MergeError(KnfError):
    path: tuple[str, ...]

class RuleError(KnfError):
    paths: tuple[tuple[str, ...], ...]

class InterpolationError(KnfError):
    problems: tuple[tuple[str, str, str], ...]

class PathError(KnfError): ...
