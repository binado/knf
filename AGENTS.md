# AGENTS.md

Guidance for AI agents working in this repository. `README.md` documents user-facing
semantics; keep it in step with any change to merge behaviour, `--set` typing,
interpolation, or error text.

## Commands

```bash
cargo test --workspace                  # everything
cargo test -p knf-core                  # fast inner loop: no filesystem, no process
cargo test -p knf-config --test library # the public API, as a consumer sees it
cargo test -p knf-cli --test cli <name> # one CLI test by name substring
cargo run -p knf-cli -- base.toml prod.toml --strict

prek run --all-files                    # cargo fmt --check + clippy -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib  # --lib: lib and bin are both `knf`
cargo insta review                      # snapshots in crates/knf/tests/snapshots/

# Dependency boundaries, checked in CI (--edges normal skips dev-deps)
cargo tree -p knf-core   --depth 1 --edges normal   # indexmap, thiserror only
cargo tree -p knf-interp --depth 1 --edges normal   # knf-core, thiserror only
cargo tree -p knf-config --depth 1 --edges normal   # never clap
```

CI runs fmt, clippy, tests, the `cargo tree` checks and `cargo doc` on Rust 1.88 and
stable, so don't use anything newer than the 1.88 MSRV. Snapshots capture multi-line
stderr where the *formatting* is under test: read the diff, don't accept it blindly.

## Crates

```
knf-core/    merge core, `Value`, path vocabulary (Seg/RefPath) — indexmap + thiserror
knf-interp/  `${...}` resolution for --interpolate — knf-core + thiserror
knf-config/  pipeline: I/O, JSON/TOML, `--set`, `merge`/`MergeOpts`. Lib name `knf`. No clap
knf/         CLI (published as knf-cli): binary only, argv and stderr
```

The split is compiler-enforced separation: `use toml::…` in the core should be a build
error. No new dependencies without a deliberate reason, and no cargo features.
Anything reusable goes in `knf-config`; `knf-cli` has no library target.

## Code rules

- **Library errors never name a CLI flag, file or layer** — only key paths. Every
  `help:` line naming a flag lives in `crates/knf/src/explain.rs`.
- Library error `Display`s end **without** a trailing newline so the CLI can append
  `help:` flush against them; `TomlError` is `#[error("{0}")]` with no `#[from]`/`#[source]`.
  CLI snapshots pin both.
- Format conversions live only in `knf-config/src/value.rs`, called only from `format.rs`.
- `ProcessEnv` (`knf-config/src/env.rs`) is the only `std::env::var` in the workspace;
  `knf-interp` gets the environment through the `Env` trait.
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
  Its JSON helper in `tests/common/mod.rs` duplicates code from `knf-config` on purpose.
- `knf-core/tests/props.rs`, `knf-interp/tests/props.rs`: proptests; strategies exclude
  floats so equality stays total.
- `knf-interp` unit tests use a `HashMap` stub `Env`, never the process environment.
- `knf-config/tests/library.rs`: public-API behaviour suite.
- `knf/tests/public_api.rs`: catches missing re-exports — name a type there once it
  becomes reachable through the public surface.
- `knf/tests/cli.rs`: runs the real binary in a tempdir; set env vars via `with_env`,
  never read the ambient environment.
