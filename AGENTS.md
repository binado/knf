# CLAUDE.md

This file provides guidance to AI agents when working with code in this repository.

## Commands

```bash
cargo test --workspace                  # everything
cargo test -p knf-core                  # fast inner loop: no filesystem, no process
cargo test -p knf-cli --test cli <name> # one CLI test by name substring
cargo run -p knf-cli -- base.toml prod.toml --strict

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
prek run --all-files                    # both of the above, per prek.toml

cargo tree -p knf-core --depth 1        # verify the core's dependency boundary
```

Snapshot tests use `insta` (`crates/knf/tests/snapshots/`). Review changes with
`cargo insta review`; these snapshots capture multi-line stderr whose *formatting*
is the thing under test, so read a diff rather than accepting it blindly.

## Architecture

Three crates, and the dependency direction is the design:

```
knf-core/     the merge core + its value type — indexmap + thiserror, nothing else
knf-dotted/   the `key.path=value` parser behind --set — thiserror, serde_json behind `json`
knf/          CLI crate, published as knf-cli (binary `knf`)
```

`knf-core` and `knf-dotted` are separate crates for **compiler-enforced separation**.
A `use clap::…` or `use toml::…` added to the core is meant to be a build error, not
a slow leak. Do not add dependencies to either without a deliberate reason — the
manifests document the rule and `cargo tree` checks it.

**One IR for every format.** `knf_core::Value` is a deliberate *superset* of JSON and
TOML: `Null` is JSON-only, `Datetime` is TOML-only. Every layer parses into it before
merging, so a JSON layer over a TOML layer needs no conversion in the middle. Format
crates appear only at the two boundaries, and the conversions live only in
`crates/knf/src/value.rs`, called only from `crates/knf/src/format.rs`.

Pipeline (`crates/knf/src/lib.rs::run`): read each positional → `format::parse` into
`Value` → append `--set` layers → `merge_with` over the flat list → `format::emit`.

### Invariants worth not breaking

- **The fold is strictly left over a flat layer list.** Merge is not associative (any
  scalar shadowing an object breaks it), so never merge subgroups and combine results.
  Flatten first, fold second. Strict mode rejects exactly the type changes that break
  associativity, so under `--strict` the merge *is* associative.
- **Arrays replace wholesale**; never index-merge or concatenate.
- **Null is an ordinary value, not a delete instruction.** This is what makes
  `knf a.json` with one argument a byte-level no-op — a property tested in
  `crates/knf-core/tests/props.rs`.
- **`Number::U64` is only for values that do not fit an `i64`.** Construct via
  `Number::from_u64`, which demotes; derived `PartialEq` would otherwise make
  `I64(1) != U64(1)` and equality would depend on which parser produced the value.
- **`Value::Datetime` may only ever be produced by the TOML parser.** It stores the
  source spelling and relies on `Display`/`FromStr` round-tripping; `value.rs`'s
  `to_toml_unchecked` has an `expect` that a new producer would make reachable.
- **Provenance never enters the merge.** `SourceName` is a CLI concept; the
  null-in-TOML error backfills filenames after the fact via
  `NullInToml::with_origins`, by re-resolving each null path against the retained
  layers. That is also why `run` only clones the layer list when output is TOML.
- **Errors in the core carry key paths and nothing else** — no filenames, no layer
  indices. Same rule in `knf-dotted`: no `--set` in its messages.
- Every input must be an object at the top level (`format::parse`).
- Output format is never guessed for mixed inputs — `-f` is required, so reordering
  arguments can never silently change the encoding.

### Tests

- `crates/knf-core/tests/cases.rs` is table-driven; adding a merge case is one line
  in `CASES`, written as JSON literals converted by `tests/common/mod.rs` (which
  duplicates ~20 lines of `knf/src/value.rs` on purpose — merge tests must not depend
  on the binary crate).
- `crates/knf-core/tests/props.rs` holds the proptest invariants above. Its value
  strategy excludes floats deliberately, so equality stays total.
- `crates/knf/tests/cli.rs` runs the real binary in a tempdir with `current_dir` set
  to the fixture, so paths in output stay relative and snapshots stay stable.

`README.md` documents the user-facing semantics; keep it in step with any change to
merge behaviour, `--set` typing, or error text.
