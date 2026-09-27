# AGENTS.md

Guidance for AI agents working in this repo:

- Commits must follow Conventional Commits.
- Keep `README.md` in sync with any changes to merge behaviour, `--set` typing, interpolation, format selection, or error text.

## Commands

MSRV is Rust 1.88.

```bash
just ci                                 # format-check + lint + test + build + doc
just test-py                            # maturin develop + pytest in active venv
just review                             # review stderr snapshots (cargo insta review)
cargo test -p knf-core --lib            # fast inner loop: unit tests only
cargo test -p knf-cli --test cli <name> # run a single CLI integration test
```

## Crates

- `knf-core/`: The library (crate name `knf`). Pipeline: `load_layers` → `merge` → `interpolate`. No `clap`.
- `knf/`: The CLI binary (`knf-cli`). Argv parsing and stderr formatting. No `pyo3`.
- `knf-py/`: Python bindings (`pyknf` wheel, module `knf._knf`). Tested via `just test-py`.

No cargo features; no new dependencies without deliberate reason. Reusable logic belongs in `knf-core`.

## Code Rules

- **Library errors never name CLI flags, files, or layers** — only key paths. Flag-specific `help:` lines belong in `crates/knf/src/explain.rs`.
- Library error `Display` implementations must **not** end with a newline (CLI appends `help:` flush against them).
- Native adapters (`serde_json`, `toml`) live strictly in `format.rs`, `value.rs`, and `set.rs`. Core algorithms (`merge.rs`, `path.rs`, `interp/`) use `ConfigValue`/`ConfigObject`/`ConfigFormat` and remain format-agnostic; tests may name native values.
- `ProcessEnv` (`knf-core/src/env.rs`) is the only `std::env::var` caller in the workspace; `interp/` uses the `Env` trait.
- Assignment/reference paths use `RefPath`; writers take keys only, validated via `RefPath::try_into_keys` prior to I/O. Merge selectors use `glob::KeyGlobPattern` against actual key segments, never diagnostic path renderings.
- TOML datetimes remain native and originate from TOML document/inline parsers; never route values through JSON.

## Invariants

- **Merge fold:** Strictly left-fold over a flat layer list (merge is not associative).
- **Arrays & Null:** Arrays replace wholesale (never merged by index or concatenated). Null is an ordinary value that overwrites, not a delete.
- **Deep by default:** Default merge is deep (`jq *`); `--shallow PATTERN` replaces matching full key paths wholesale. `*` is `jq +` at the root; `foo.*` is shallow inside `foo`, while `foo` replaces it entirely. Dots separate keys; single-quoted spans are literal. Matching ancestors stop traversal.
- **Interpolation:** Opt-in, runs once over the merged document. Env values are terminal; container references are whole-string only.
- **Native representation:** JSON nulls/unsigned integers and TOML datetimes/non-finite floats stay native. Inline and whole-string environment values parse in the selected format with original-text string fallback.
- **Input/Output:** Every input is a top-level object/table. Inferred mixed formats fail before document I/O; `-f` overrides parsing for every input and selects that same output format. An empty input list defaults to JSON.
- **Python datetimes:** Return datetime/date/time objects with fixed offsets when present; truncate nanoseconds to microseconds, and report unrepresentable datetime values with key paths.

## Tests

- `knf-core/tests/cases.rs`: Table-driven merge test cases.
- `knf-core/tests/*props.rs`: Proptests (floats excluded to ensure total equality).
- `knf-core/tests/library.rs`: Public library API suite.
- `knf/tests/public_api.rs`: Guards against missing re-exports.
- `knf/tests/cli.rs`: CLI integration tests (use `with_env`, never ambient env).
- `interp/` unit tests: Must use `HashMap` stub `Env`, never process env.
