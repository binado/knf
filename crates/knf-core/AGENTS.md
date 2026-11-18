# AGENTS.md (knf-core)

The library crate (`knf`). Pipeline: `load_layers` → `merge`, or `load_layers` → `merge_interpolate` with interpolation. No `clap`.

## Code Rules

- **Errors never name CLI flags, files, or layers**, only key paths. Flag-specific `help:` lines belong in `crates/knf/src/explain.rs`.
- Error `Display` implementations must **not** end with a newline (the CLI appends `help:` flush against them).
- Core algorithms (`merge.rs`, `path.rs`, `interp/`) use `ConfigValue`/`ConfigObject`/`ConfigFormat` and stay format-agnostic.
- `interp/` reads the environment only through the `Env` trait.
- Assignment/reference paths use `RefPath`; writers take keys only, validated via `RefPath::try_into_keys` prior to I/O. Merge selectors use `glob::KeyGlobPattern` against actual key segments, never diagnostic path renderings.

## Invariants

- **Merge fold:** Strictly left-fold over a flat layer list (merge is not associative).
- **Arrays & Null:** Arrays replace wholesale (never merged by index or concatenated). Null is an ordinary value that overwrites, not a delete.
- **Deep by default:** Default merge is deep (`jq *`); `--shallow PATTERN` replaces matching full key paths wholesale. `*` is `jq +` at the root; `foo.*` is shallow inside `foo`, while `foo` replaces it entirely. Dots separate keys; single-quoted spans are literal. Matching ancestors stop traversal.
- **Interpolation:** Opt-in. Layers fold as an expression graph (`interp/graph.rs`): a whole-string reference merges exactly as the value it names, and references bind to the final document. Replaced operands never resolve; a reference whose kind decides a merge must. Markers fold last-wins and apply once. Native `merge` is the path without interpolation. Env values are terminal; container references are whole-string only.
- **Native representation:** JSON nulls/unsigned integers and TOML datetimes/non-finite floats stay native. Inline and whole-string environment values parse in the selected format with original-text string fallback.
- **Input:** Every input is a top-level object/table. An empty input list defaults to JSON.

## Tests

- `tests/cases.rs`: Table-driven merge test cases.
- `tests/*props.rs`: Proptests (floats excluded to ensure total equality).
- `tests/library.rs`: Public library API suite.
- `interp/` unit tests: Must use a `HashMap` stub `Env`, never process env.
