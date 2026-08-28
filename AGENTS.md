# CLAUDE.md

This file provides guidance to AI agents when working with code in this repository.

## Commands

```bash
cargo test --workspace                  # everything
cargo test -p knf-core                  # fast inner loop: no filesystem, no process
cargo test -p knf-config --test library # the public API, as a consumer sees it
cargo test -p knf-cli --test cli <name> # one CLI test by name substring
cargo run -p knf-cli -- base.toml prod.toml --strict

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
prek run --all-files                    # both of the above, per prek.toml

# --lib because knf-config's lib and knf-cli's bin are both named `knf` and
# rustdoc writes both to target/doc/knf; the binary has no API to document.
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib

# The dependency direction is the design; CI runs these three as a job.
# --edges normal excludes dev-deps (knf-core's table tests take serde_json).
cargo tree -p knf-core   --depth 1 --edges normal   # indexmap, thiserror. That is all.
cargo tree -p knf-interp --depth 1 --edges normal   # knf-core, thiserror — no serde_json
cargo tree -p knf-config --depth 1 --edges normal   # formats and I/O, but never clap
```

Snapshot tests use `insta` (`crates/knf/tests/snapshots/`). Review changes with
`cargo insta review`; these snapshots capture multi-line stderr whose *formatting*
is the thing under test, so read a diff rather than accepting it blindly.

## Architecture

Four crates, and the dependency direction is the design:

```
knf-core/     the merge core + its value type + the path vocabulary
              (Seg/RefPath) — indexmap + thiserror, nothing else
knf-interp/   `${key.path}` / `${servers[0]}` / `${env:VAR}` resolution behind
              --interpolate — knf-core, thiserror
knf-config/   the pipeline: file I/O, JSON and TOML, `key.path=value`, and the
              `merge`/`MergeOpts` API. Library name `knf`. No clap
knf/          CLI crate, published as knf-cli — the binary `knf` and nothing else
```

The crates are separate for **compiler-enforced separation**. A `use clap::…` or
`use toml::…` added to the core is meant to be a build error, not a slow leak — the
reason the boundary is a dependency edge rather than a cargo feature, which would
make it a `#[cfg]` and a convention. Do not add dependencies to any of them without
a deliberate reason: the manifests document the rule and CI runs `cargo tree`.

`knf-cli` has **no library target**. Everything reusable is `knf-config`, which is
what a Rust consumer, and any future language binding, depends on; the binary crate
is argv and stderr. That also means there are no cargo features anywhere in the
workspace.

`knf-interp` depends on `knf-core` alone, which is what keeps `serde_json` out of its
tree — the path vocabulary comes from a crate that has none either. It also contains
**no `std::env`**: the environment arrives through the `Env` trait, which is what
keeps it deterministic and testable without touching process state, and what keeps
the JSON-or-string typing rule out of it. `ProcessEnv`
(`crates/knf-config/src/env.rs`) is the workspace's only `std::env::var`, and it
calls `set::json_or_string` so `${env:PORT}` types exactly as `--set port=…` does.
`merge_with_env` exposes that same seam to a caller, so a library consumer is not
forced through process state either.

**Path types: one vocabulary, one spelling, one predicate.** `Seg` is the single
step type. `RefPath` is the one parsed spelling (`a.b[2].c`) — references and
the write-side flags share the grammar, since reading an array element and
malformed-bracket rejection want the same parser. A bare `Vec<Seg>` is the
witness a walker builds. Writers take keys only — arrays replace wholesale, so
an index can never *write* — and that one predicate lives in
`RefPath::try_into_keys`, run once per flag at the boundary (`--set` expansion,
`merge_opts`), before any I/O, rather than being carried by a separate type.
A key literally spelled `a[0]` is consequently unwritable from the command line
and unreferenceable from `${...}`; only a file can carry one.

The vocabulary lives in `knf-core/src/path.rs` — pure text handling that needs
nothing the core does not already have, and putting it beside `Value` is what keeps
one spelling and one renderer for the paths both merge errors and `${...}`
references display. Two renderers, and the split is real: `render_path` takes a
witness (`&[Seg]`, possibly holding indices) and renders an empty one as nothing,
because a parsed `RefPath` can never be empty; `render_keys` takes the merge side's
`&[String]` and renders empty as `<root>`, which is reachable there. `lookup` sits
in `knf-interp` because that crate is its only caller.

`PathLeaf` and `json_or_string` are in `knf-config/src/set.rs`, not with the rest of
the vocabulary, and the compiler is why: `impl FromStr for PathLeaf<serde_json::Value>`
and its siblings name no local type unless `PathLeaf` is local, so the JSON impls
and the struct cannot be separated — and `knf-core` must not gain `serde_json`.

**One IR for every format.** `knf_core::Value` is a deliberate *superset* of JSON and
TOML: `Null` is JSON-only, `Datetime` is TOML-only. Every layer parses into it before
merging, so a JSON layer over a TOML layer needs no conversion in the middle. Format
crates appear only at the two boundaries, and the conversions live only in
`crates/knf-config/src/value.rs`, called only from `crates/knf-config/src/format.rs`.

Pipeline (`crates/knf/src/main.rs::run`): build `MergeOpts` — rules and every `--set`
expression, so a mistake in argv fails before any I/O → `load_layers` (read each
positional, `format::parse` into `Value`) → `resolve_output_format` → `merge_layers`
(append the overlays, `merge_with` over the flat list, `knf_interp::interpolate` if
`--interpolate`) → `format::emit`. `knf_config::merge` is the library entry point onto
the same two halves, minus the format decision.

**The output format is resolved between the parse and the fold**, and the split into
`load_layers`/`merge_layers` exists to hold that ordering. A missing `-f` is a mistake
in argv alone; deciding it after the merge would queue it behind every error in the
documents themselves, so the user would fix a type conflict, re-run, and only then
learn about the flag — the same "one run at a time" pattern the rule-conflict message
is built to avoid. Two CLI snapshots pin it.

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
  A consequence at the TOML boundary: since the constructor demotes, a *canonical*
  `U64` always holds a value past `i64::MAX`, which TOML's signed integers cannot
  spell — so `to_toml` rejects it rather than rounding through `f64`, which would
  discard the exact digits the variant exists to keep. `collect_untomlable` guards
  on the range rather than the variant, so a hand-built `U64(1)` still converts.
- **`Value::Datetime` may only ever *originate* in the TOML parser.** It stores the
  source spelling and relies on `Display`/`FromStr` round-tripping.
  Interpolation may **copy** one (`d2 = "${d}"` takes the referent's type) — sound,
  since the string still round-trips — but nothing may ever **synthesize** one from
  text. That is a second reason `${env:...}` types through JSON, which has no datetime
  and so structurally cannot fabricate one. The rule is a convention, not a type: the
  variant is public and holds a plain `String`, so a caller assembling a `Value` by
  hand can break it, and enforcing it at construction would need `toml` inside
  `knf-core`. `to_toml`'s pre-walk therefore checks the spelling and reports
  `BadDatetime` where an `expect` used to abort — a backstop for the one producer the
  compiler cannot rule out, not a licence to add another.
- **No layer outlives the merge.** `merge_layers` folds a plain `Vec<Value>`; `SourceName`
  names an input only while it is being *read*, for parse errors, which is why it has
  no `--set` variant. The null-in-TOML error therefore carries key paths and no
  filenames — retaining every parsed layer past the merge to attribute a rare error is
  not worth the clone, and neither escape it offers (`-f json`, `--null-as`)
  needs to know which file the null came from.
- **What a format cannot spell is rejected by path, never substituted.** The one rule
  behind every emission error, and it runs in both directions. `collect_untomlable`
  carries TOML's three impossibilities — nulls, integers past `i64::MAX`, unparseable
  datetimes — reported in that order, which is how close each is to something a real
  input file can contain: a null and an oversized integer arrive from a document, a bad
  datetime only from a caller. `collect_unjsonable` carries JSON's one, a non-finite
  float, which an ordinary `.toml` input supplies because TOML's grammar has `inf`,
  `-inf` and `nan` literals and JSON's has none. **The two sets do not overlap, so each
  format is the escape from the other's rejection** — which is what the `help:` lines
  say, and why neither library error has to. `to_json` returns `NonFiniteFloat`
  directly rather than an enum: JSON has exactly one impossibility, and a
  single-variant enum costs a consumer a `match` and buys nothing. Every one of these
  used to substitute silently — a null vanished, `inf` became `0`, a snowflake ID
  became `1e19` — which is the class of bug this pre-walk exists to make impossible.
  Collecting up front rather than failing inside the conversions is also what keeps
  `to_toml_unchecked` and `to_json_unchecked` infallible: no `Result` threads through
  their array and table arms, and the reports carry key paths. `--null-as` is the one
  escape that is not simply the other format, and it substitutes rather than drops because a null inside an array
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
- **No library crate names a command-line flag.** Errors in the core carry key paths
  and nothing else — no filenames, no layer indices, no flag names: `Locked` and
  `AppendKind` must not say `--fail` or `--append`. Same rule in `knf-interp` (no
  `--interpolate`) and in `knf-config`, which is why the three load failures that used
  to say `--input-format` are the typed `LoadError` instead, and why `NullInToml`
  reports *where* the nulls are and leaves `-f json` and `--null-as` unsaid.
  `IntegerOutOfRange` and `NonFiniteFloat` keep the same silence about `-f json` and
  `-f toml`; `BadDatetime` names nothing at all, there being no flag that would help.
  Every `help:` line naming a flag lives in `crates/knf/src/explain.rs` and nowhere else,
  reached by the one `explain_pipeline` every stage's errors pass through; library
  tests assert a `LoadError` contains no `--` and neither TOML report a flag.
  `knf-interp` cannot name a file even if it wanted to — it runs after the merge, and
  no layer outlives the merge.
- **A library error that ends without a newline is a seam, not an oversight.**
  `NullInToml`'s `Display` stops after its last `-->` line precisely so `knf-cli` can
  append a `help:` line flush against it; restoring the `writeln!` would put a blank
  line in the CLI's stderr, which `cli__null_in_toml_error` pins. `BadDatetime`,
  `IntegerOutOfRange` and `NonFiniteFloat` keep the same shape, pinned by
  `cli__integer_out_of_range_in_toml_error` and `cli__non_finite_in_json_error`. The `TomlError` wrapping them is `#[error("{0}")]` with **no**
  `#[from]` or `#[source]` for the same reason: an auto-derived `source()` would be a
  second copy of a message the variant already prints in full, and `main.rs` walks the
  cause chain onto stderr.
- Every input must be an object at the top level (`format::parse`).
- Output format is never guessed for mixed inputs — `-f` is required, so reordering
  arguments can never silently change the encoding.

### Tests

- `crates/knf-core/tests/cases.rs` is table-driven; adding a merge case is one line
  in `CASES`, written as JSON literals converted by `tests/common/mod.rs` (which
  duplicates ~20 lines of `knf-config/src/value.rs` on purpose — merge tests must not
  depend on the crate that knows about formats). A `Case` holds `strict` and a `rules` slice rather than a
  `MergeOptions`, so the table stays `const` and one line per case.
- `crates/knf-core/tests/props.rs` holds the proptest invariants above. Its value
  strategy excludes floats deliberately, so equality stays total.
- `crates/knf-interp/tests/props.rs` holds the identity property, over its own small
  `arb_value` — test-only generators do not cross crates, so it duplicates the shape of
  the one above (and the same float exclusion) on purpose.
- `crates/knf-interp` unit tests run against a `HashMap`-backed stub `Env`, never the
  process environment.
- `crates/knf-config/tests/library.rs` exercises the public API the way a consumer
  would, and is the dedicated behaviour suite for `merge`, `merge_with_env` and
  `LoadError`. It cannot catch a missing `pub use`, though — an integration test sees
  its own package's dependencies, so `knf_core::Number` resolves there whether or not
  `knf-config` re-exports it.
- `crates/knf/tests/public_api.rs` is what catches that instead, by being the
  downstream crate: `knf-cli` depends on `knf-config` and on neither `knf-core` nor
  `knf-interp`, so every `knf::` path in it resolves through a re-export or fails to
  compile. Name a type there when it becomes reachable *through* the public surface,
  not only when a signature mentions it — `Number` behind `Value::Number`, `Cycle`
  and `Syntax` behind `InterpError`, `render_path` for the `Seg`s `RefPath` yields.
- `crates/knf/tests/cli.rs` runs the real binary in a tempdir with `current_dir` set
  to the fixture, so paths in output stay relative and snapshots stay stable. Anything
  touching `${env:...}` sets its variables explicitly through the `with_env` helper
  (`Command::env`/`env_remove`), so no test reads the ambient environment.

`README.md` documents the user-facing semantics; keep it in step with any change to
merge behaviour, `--set` typing, interpolation, or error text.
