#!/usr/bin/env bash
# Builds the `knf` binary and stages it in maturin's `data` directory, which the
# `knf-cli` wheel installs as a script. Run from the repository root, before
# `maturin build` / `maturin develop`. PEP 517 paths (`pip wheel .`,
# `python -m build`) go through `knf_build.py`, which does this itself.
# maturin refuses to build if the data directory is missing, so a wheel
# without the executable cannot be published by accident.
#
#   crates/knf-py/stage-cli.sh                       # host target
#   crates/knf-py/stage-cli.sh aarch64-apple-darwin  # an explicit target
set -euo pipefail

if [[ $# -gt 0 ]]; then
  cargo build --release --locked -p knf-cli --target "$1"
  out="target/$1/release"
else
  cargo build --release --locked -p knf-cli
  out="target/release"
fi

exe=knf
[[ -f "$out/knf.exe" ]] && exe=knf.exe

mkdir -p crates/knf-py/data/scripts
cp "$out/$exe" crates/knf-py/data/scripts/
