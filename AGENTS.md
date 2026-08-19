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
cargo tree -p knf-interp --depth 1      # knf-core, knf-dotted, thiserror — no serde_json
```

Snapshot tests use `insta` (`crates/knf/tests/snapshots/`). Review changes with
`cargo insta review`; these snapshots capture multi-line stderr whose *formatting*
is the thing under test, so read a diff rather than accepting it blindly.

## Architecture

Four crates, and the dependency direction is the design:

```
knf-core/     the merge core + its value type — indexmap + thiserror, nothing else
knf-dotted/   the path vocabulary (Seg/KeyPath/RefPath) + the `key.path=value`
              parser behind --set — thiserror, serde_json behind `json`
knf-interp/   `${key.path}` / `${servers[0]}` / `${env:VAR}` resolution behind
              --interpolate — knf-core, knf-dotted (paths only), thiserror
knf/          CLI crate, published as knf-cli (binary `knf`)
```

The library crates are separate for **compiler-enforced separation**. A `use clap::…`
or `use toml::…` added to the core is meant to be a build error, not a slow leak. Do
not add dependencies to any of them without a deliberate reason — the manifests
document the rule and `cargo tree` checks it.

`knf-interp` takes `knf-dotted` with `default-features = false`, set on the
*workspace* dependency because a member cannot turn a workspace default back off.
That keeps `serde_json` out of its tree: `Seg`, `KeyPath` and `RefPath` are
unconditional in `knf-dotted/src/lib.rs`, only `json.rs` is gated. It also
contains **no `std::env`** — the environment arrives through the `Env` trait,
which is what keeps it deterministic and testable without touching process
state, and what keeps the JSON-or-string typing rule out of it. `ProcessEnv`
(`crates/knf/src/interp.rs`) is the workspace's only `std::env::var`, and it
calls `knf_dotted::json_or_string` so `${env:PORT}` types exactly as
`--set port=…` does.

**Path types: one vocabulary, two predicates.** `Seg` is the single step type.
`KeyPath` is a newtype over `Vec<Seg>` whose only constructors keep it all-key —
merge-side paths (`--set`, the rule flags) can *never* be indexed, by
construction rather than discipline. A bare `Vec<Seg>` is the witness a walker
builds; `RefPath` is the one bracket-accepting spelling, for a consumer that
only reads. `lookup` stays out of `knf-dotted` because it needs
`knf_core::Value`, and `knf-dotted` must not.

**One IR for every format.** `knf_core::Value` is a deliberate *superset* of JSON and
TOML: `Null` is JSON-only, `Datetime` is TOML-only. Every layer parses into it before
merging, so a JSON layer over a TOML layer needs no conversion in the middle. Format
crates appear only at the two boundaries, and the conversions live only in
`crates/knf/src/value.rs`, called only from `crates/knf/src/format.rs`.

Pipeline (`crates/knf/src/lib.rs::run`): build `MergeOptions` (so a bad rule set fails
before any I/O) → read each positional → `format::parse` into `Value` → append `--set`
layers → `merge_with` over the flat list → `knf_interp::interpolate` if `--interpolate`
→ `format::emit`.

**Interpolation runs once, on the merged document, never per layer.** Several
consequences fall out of that placement and need no code: `--set` layers interpolate
like any other layer; `--strict` validates types *before* substitution, so a `"${port}"`
was a string when it looked; `--null-as` is not interpolated (it runs later, and is a
literal from argv); and a reference resolving to `Null` meets the existing TOML-null
error and its existing escape.

**Per-path strategies** (`knf-core/src/rules.rs`) live in the core because `merge_at`
already threads the key path and the rule trie narrows on the same descent — pure data,
no new dependencies. Flag *parsing* stays in `crates/knf/`, and so does every mention of
`--append`, `--replace` and `--fail`: the core knows only `Strategy` names.

### Invariants worth not breaking

- **The fold is strictly left over a flat layer list.** Merge is not associative (any
  scalar shadowing an object breaks it), so never merge subgroups and combine results.
  Flatten first, fold second. Strict mode rejects exactly the type changes that break
  associativity, so under `--strict` the merge *is* associative.
- **Arrays replace wholesale** unless `--append` names the path; never index-merge, and
  never concatenate anywhere else.
- **Rules are a set, not a list.** Flag order must never affect the output — the same
  value `resolve_output_format` protects. `Rules::build` validates the finished set in
  one pass (rather than an insert-time check) and reports every offender sorted, so an
  illegal set produces an identical message whatever order the flags arrived in. A
  conflict reports the *whole* set at that path, not a pair: three flags on one path is
  one error naming all three, so the user never learns of them one run at a time.
- **Terminal nesting is rejected up front.** Every `Strategy` stops the walk, so a rule
  beneath another can never fire and is an error when the set is built — before any file
  is read. The rationale is structural rather than a carve-out: the default merge is the
  *absence* of a rule, not a variant, so there is nothing a deeper rule could sit under.
  That is also why there is no `--merge` flag — without globs it could only ever be
  redundant or unreachable.
- **A strategy only applies where the accumulator already holds a value**; an absent key
  is inserted regardless. That is what keeps `Fail` meaning "the first layer to define
  this pins it" and keeps `Append` from doubling a lone layer's array against the empty
  seed — see the identity property in `props.rs`.
- **Null is an ordinary value, not a delete instruction.** This is what makes
  `knf a.json` with one argument a byte-level no-op — a property tested in
  `crates/knf-core/tests/props.rs`.
- **`Number::U64` is only for values that do not fit an `i64`.** Construct via
  `Number::from_u64`, which demotes; derived `PartialEq` would otherwise make
  `I64(1) != U64(1)` and equality would depend on which parser produced the value.
- **`Value::Datetime` may only ever *originate* in the TOML parser.** It stores the
  source spelling and relies on `Display`/`FromStr` round-tripping; `value.rs`'s
  `to_toml_unchecked` has an `expect` that a new producer would make reachable.
  Interpolation may **copy** one (`d2 = "${d}"` takes the referent's type) — sound,
  since the string still round-trips — but nothing may ever **synthesize** one from
  text. That is a second reason `${env:...}` types through JSON, which has no datetime
  and so structurally cannot fabricate one.
- **No layer outlives the merge.** `run` folds a plain `Vec<Value>`; `SourceName`
  names an input only while it is being *read*, for parse errors, which is why it has
  no `--set` variant. The null-in-TOML error therefore carries key paths and no
  filenames — retaining every parsed layer past the merge to attribute a rare error is
  not worth the clone, and neither escape it offers (`-f json`, `--null-as`)
  needs to know which file the null came from.
- **Null is rejected on the way to TOML, never dropped.** The `toml` crate's map
  serializer silently *skips* a `None` entry, so `value.rs`'s `collect_nulls` pre-walk is
  the only thing standing between a null and quietly-missing keys. `--null-as` is
  the one escape, and it substitutes rather than drops because a null inside an array
  cannot be removed without shifting every index after it — `yq` and `tomlq` both fabricate
  a string there instead, and not the same one. It applies to TOML emission only (in
  `format::emit`): JSON holds a null fine, so there is nothing for it to rescue.
- **Interpolation is opt-in, and that is load-bearing rather than a preference.**
  `knf a.json` is documented and proptested as a byte-level no-op, and knf sits directly
  upstream of tools whose own syntax is `${...}` — compose files, Actions workflows, Helm
  charts, systemd units. Eating those by default would be silent corruption. Opt-in also
  keeps "output is a function of the inputs alone" a guarantee rather than a default,
  which matters once `${env:}` makes stdout depend on the ambient environment. The
  identity property lives in `crates/knf-interp/tests/props.rs`.
- **Environment values are terminal.** A variable is never re-scanned, so it cannot
  reach back into the document; only document references resolve transitively. Embedded,
  a variable splices its **raw** text — parsing it and re-rendering it could only corrupt
  it — while whole-string it is typed by the caller's rule.
- **A container reference is whole-string only.** `alias = "${db}"` aliases the (fully
  resolved) subtree; `url = "x/${db}"` is an error. There is no format-independent answer
  to how an object renders inside a string — `{host = "x"}` under `-f toml`,
  `{"host":"x"}` under `-f json` — and answering it would make `knf-interp` know the
  output format and pull in a format crate, a third place they appear. Rejecting is the
  loosenable direction.
- **`env:` is a literal prefix match, not a split on the first `:`.** So `${a:b}` is the
  key `a:b` and works, and `${db.host:port}` does not produce a baffling "unknown
  namespace `db.host`". The only unaddressable keys are those literally beginning `env:`.
  A second namespace added later would change meaning for such a document; accepted
  knowingly.
- **Errors in the core carry key paths and nothing else** — no filenames, no layer
  indices, and no flag names: `Locked` and `AppendKind` must not say `--fail` or
  `--append`. `crates/knf/src/lib.rs` adds the `help:` line naming the flag. Same rule
  in `knf-dotted` (no `--set`) and in `knf-interp` (no `--interpolate`; `explain_interp`
  sits next to `explain_rules` and `name_the_flag`). `knf-interp` cannot name a file even
  if it wanted to — it runs after the merge, and no layer outlives the merge.
- Every input must be an object at the top level (`format::parse`).
- Output format is never guessed for mixed inputs — `-f` is required, so reordering
  arguments can never silently change the encoding.

### Tests

- `crates/knf-core/tests/cases.rs` is table-driven; adding a merge case is one line
  in `CASES`, written as JSON literals converted by `tests/common/mod.rs` (which
  duplicates ~20 lines of `knf/src/value.rs` on purpose — merge tests must not depend
  on the binary crate). A `Case` holds `strict` and a `rules` slice rather than a
  `MergeOptions`, so the table stays `const` and one line per case.
- `crates/knf-core/tests/props.rs` holds the proptest invariants above. Its value
  strategy excludes floats deliberately, so equality stays total.
- `crates/knf-interp/tests/props.rs` holds the identity property, over its own small
  `arb_value` — test-only generators do not cross crates, so it duplicates the shape of
  the one above (and the same float exclusion) on purpose.
- `crates/knf-interp` unit tests run against a `HashMap`-backed stub `Env`, never the
  process environment.
- `crates/knf/tests/cli.rs` runs the real binary in a tempdir with `current_dir` set
  to the fixture, so paths in output stay relative and snapshots stay stable. Anything
  touching `${env:...}` sets its variables explicitly through the `with_env` helper
  (`Command::env`/`env_remove`), so no test reads the ambient environment.

`README.md` documents the user-facing semantics; keep it in step with any change to
merge behaviour, `--set` typing, interpolation, or error text.
