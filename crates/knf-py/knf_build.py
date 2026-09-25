"""PEP 517 backend: stage the `knf` executable, then hand off to maturin.

maturin cannot build a `bin` target next to a pyo3 module, so the wheel's
`knf` comes from `knf-cli` copied into `data/scripts/`. Direct `maturin build`
still needs `stage-cli.sh`; `pip wheel .` / `python -m build` go through here.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Mapping, Optional

from maturin import (
    build_editable as _build_editable,
    build_sdist as _build_sdist,
    build_wheel as _build_wheel,
    get_maturin_pep517_args,
    get_requires_for_build_editable,
    get_requires_for_build_sdist,
    get_requires_for_build_wheel,
    prepare_metadata_for_build_editable as _prepare_metadata_for_build_editable,
    prepare_metadata_for_build_wheel as _prepare_metadata_for_build_wheel,
)

__all__ = [
    "build_editable",
    "build_sdist",
    "build_wheel",
    "get_requires_for_build_editable",
    "get_requires_for_build_sdist",
    "get_requires_for_build_wheel",
    "prepare_metadata_for_build_editable",
    "prepare_metadata_for_build_wheel",
]

_SCRIPTS = Path("crates") / "knf-py" / "data" / "scripts"


def _project_root() -> Path:
    here = Path(__file__).resolve().parent
    for candidate in (here, *here.parents):
        if (candidate / "pyproject.toml").is_file() and (
            candidate / "crates" / "knf" / "Cargo.toml"
        ).is_file():
            return candidate
    raise RuntimeError("could not locate the knf repository root")


def _cli_target(config_settings: Optional[Mapping[str, Any]] = None) -> Optional[str]:
    target = os.environ.get("KNF_CLI_TARGET") or os.environ.get("CARGO_BUILD_TARGET") or None
    args = get_maturin_pep517_args(config_settings)
    index = 0
    while index < len(args):
        arg = args[index]
        if arg == "--target" and index + 1 < len(args):
            target = args[index + 1]
            index += 2
            continue
        if arg.startswith("--target="):
            target = arg.split("=", 1)[1]
        index += 1
    return target or None


def _cargo_env() -> Optional[dict[str, str]]:
    if os.environ.get("MATURIN_NO_INSTALL_RUST") or shutil.which("cargo"):
        return None
    # puccinialin is installed after get_requires_for_build_wheel, so it is
    # not importable when this module is first loaded.
    from puccinialin import setup_rust

    print("Rust not found, installing into a temporary directory")
    extra_env = setup_rust()
    return {**os.environ, **extra_env}


def _ensure_data_dir(root: Path) -> Path:
    dest = root / _SCRIPTS
    dest.mkdir(parents=True, exist_ok=True)
    return dest


def stage_cli(config_settings: Optional[Mapping[str, Any]] = None) -> None:
    """Build knf-cli and copy it to maturin's `data/scripts/` directory."""
    root = _project_root()
    target = _cli_target(config_settings)
    cmd = ["cargo", "build", "--release", "--locked", "-p", "knf-cli"]
    if target:
        cmd.extend(["--target", target])
    print("Staging knf executable:", " ".join(cmd))
    sys.stdout.flush()
    subprocess.check_call(cmd, cwd=root, env=_cargo_env())
    out = root / "target" / target / "release" if target else root / "target" / "release"
    exe = "knf.exe" if (out / "knf.exe").is_file() else "knf"
    src = out / exe
    if not src.is_file():
        raise FileNotFoundError(f"cargo did not produce {src}")
    dest = _ensure_data_dir(root)
    shutil.copy2(src, dest / exe)


def build_wheel(
    wheel_directory: str,
    config_settings: Optional[Mapping[str, Any]] = None,
    metadata_directory: Optional[str] = None,
) -> str:
    stage_cli(config_settings)
    return _build_wheel(wheel_directory, config_settings, metadata_directory)


def build_editable(
    wheel_directory: str,
    config_settings: Optional[Mapping[str, Any]] = None,
    metadata_directory: Optional[str] = None,
) -> str:
    stage_cli(config_settings)
    return _build_editable(wheel_directory, config_settings, metadata_directory)


def prepare_metadata_for_build_wheel(
    metadata_directory: str,
    config_settings: Optional[Mapping[str, Any]] = None,
) -> str:
    # write-dist-info requires the configured data directory to exist; the
    # binary itself is not part of the metadata.
    _ensure_data_dir(_project_root())
    return _prepare_metadata_for_build_wheel(metadata_directory, config_settings)


def prepare_metadata_for_build_editable(
    metadata_directory: str,
    config_settings: Optional[Mapping[str, Any]] = None,
) -> str:
    _ensure_data_dir(_project_root())
    return _prepare_metadata_for_build_editable(metadata_directory, config_settings)


def build_sdist(
    sdist_directory: str,
    config_settings: Optional[Mapping[str, Any]] = None,
) -> str:
    # Do not copy a platform binary into the sdist; gitignore keeps `data/` out.
    _ensure_data_dir(_project_root())
    return _build_sdist(sdist_directory, config_settings)
