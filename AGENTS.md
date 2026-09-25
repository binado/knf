# AGENTS.md

Guidance for AI agents working in this repository. `README.md` documents user-facing
semantics; keep it in step with any change to merge behaviour, `--set` typing,
interpolation, or error text.

## Commands

```bash
cargo test --workspace                  # everything
cargo test -p knf-core --lib            # fast inner loop: unit tests only
cargo test -p knf-core --test library   # the public API, as a consumer sees it
cargo test -p knf-cli --test cli <name> # one CLI test by name substring
cargo run -p knf-cli -- base.toml prod.toml --strict

prek run --all-files                    # cargo fmt --check + clippy -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib  # --lib: lib and bin are both `knf`
cargo insta review                      # snapshots in crates/knf/tests/snapshots/

cargo tree -p knf-core --depth 1 --edges normal     # never clap (checked in CI)
cargo tree -p knf-cli --edges normal                # never pyo3 (checked in CI)

# Python: build the knf-cli wheel into a venv, then test it as installed
# `pip wheel .` / `python -m build` stage the knf binary via the PEP 517 backend.
# `maturin develop` still needs it staged first:
crates/knf-py/stage-cli.sh
maturin develop && pytest crates/knf-py/tests
```

CI runs fmt, clippy, tests, the `cargo tree` check and `cargo doc` on Rust 1.88 and
stable, so don't use anything newer than the 1.88 MSRV. Snapshots capture multi-line
stderr where the *formatting* is under test: read the diff, don't accept it blindly.

## Crates

```
knf-core/    the library, lib name `knf`. No clap
  ir.rs        `Value`/`Map`/`Number`
  merge.rs     the fold: `merge`, `merge_into`, `MergeOptions`, strict mode
  path.rs      path vocabulary (`Seg`/`RefPath`) and `lookup`
  interp/      `${...}` resolution for --interpolate
  format.rs    detect/parse/emit; value.rs: JSON/TOML <-> IR; set.rs: `key.path=value`
  lib.rs       `load_layers` (I/O) and the re-exports
knf/         CLI (published as knf-cli): binary only, argv and stderr
knf-py/      Python module `knf._knf` (pyo3, never on crates.io): arguments and exceptions
```

The `knf-cli` **wheel** is built from `knf-py` (root `pyproject.toml`): the pyo3
module, plus knf-cli's binary that the PEP 517 backend (`knf_build.py`) — or
`stage-cli.sh` before a direct `maturin` invocation — puts in maturin's
`data/scripts/`, because maturin can't build a bin next to a pyo3 module.
`knf-py` has no Rust tests: its tests are `knf-py/tests/*.py`, and they run
against an installed wheel. Building it with plain cargo needs
`PYO3_BUILD_EXTENSION_MODULE=1` (set in CI) unless libpython is installed.

The public API is three composable steps: `load_layers` → `merge` → `interpolate`.
No new dependencies without a deliberate reason, and no cargo features. Anything
reusable goes in `knf-core`; `knf-cli` has no library target; pyo3 appears only in
`knf-py`.

## Code rules

- **Library errors never name a CLI flag, file or layer** — only key paths. Every
  `help:` line naming a flag lives in `crates/knf/src/explain.rs`.
- Library error `Display`s end **without** a trailing newline so the CLI can append
  `help:` flush against them; `TomlError` is `#[error("{0}")]` with no `#[from]`/`#[source]`.
  CLI snapshots pin both.
- `serde_json`/`toml` appear only in `format.rs`, `value.rs` and `set.rs`; `merge.rs`,
  `ir.rs`, `path.rs` and `interp/` never name a format. Format conversions live only in
  `value.rs`, called only from `format.rs`.
- `ProcessEnv` (`knf-core/src/env.rs`) is the only `std::env::var` in the workspace;
  `interp/` gets the environment through the `Env` trait.
- One path grammar (`RefPath`); writers take keys only, checked once via
  `RefPath::try_into_keys` before any I/O.
- Construct `Number::U64` via `Number::from_u64` (it demotes to `I64` when it fits).
- `Value::Datetime` may only originate in the TOML parser — never synthesize one from text.

## Invariants

- The fold is strictly left over a flat layer list; merge is not associative, so never
  merge subgroups and combine.
- Arrays replace wholesale. Null is an ordinary value, not a delete.
  `knf a.json` is a byte-level no-op (proptested).
- Default merge is deep (jq `*`); `--shallow` is jq `+`. No per-path rules.
- Interpolation is opt-in and runs once, on the merged document. Env values are
  terminal (never re-scanned); container references are whole-string only.
- Values a format cannot represent (TOML: null, ints past `i64::MAX`, bad datetimes;
  JSON: non-finite floats) are rejected by path, never silently substituted.
- Every input is an object at top level; output format is never guessed for mixed
  inputs (`-f` required), and is resolved before the merge so argv errors come first.

## Tests

- `knf-core/tests/cases.rs`: table-driven, one line per merge case in `CASES`.
- `knf-core/tests/props.rs`, `knf-core/tests/interp_props.rs`: proptests; strategies
  exclude floats so equality stays total.
- `interp/` unit tests use a `HashMap` stub `Env`, never the process environment.
- `knf-core/tests/library.rs`: public-API behaviour suite.
- `knf/tests/public_api.rs`: catches missing re-exports — name a type there once it
  becomes reachable through the public surface.
- `knf/tests/cli.rs`: runs the real binary in a tempdir; set env vars via `with_env`,
  never read the ambient environment.
- `knf-py/tests/test_deep_merge.py`: pytest against the installed wheel, including
  the bundled `knf` executable.
