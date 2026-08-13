# knf — config merge tool

**Status:** design, pre-implementation.

## 1. Idea

`knf` merges layered configuration files and prints the result. One job, no
query language, no templating.

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json --set server.port=8080
```

It exists because the alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

JSON and TOML layers can be mixed freely. Every format parses into one owned
value tree; the format crates are involved only at parse and emit.

---

## 2. Core model

Every format deserializes into one owned IR, a superset of both data models:

```rust
enum Value {
    Null,                 // JSON only
    Bool(bool),
    Number(Number),       // I64 | U64 | F64
    String(String),
    Datetime(String),     // TOML only; the source spelling
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}
```

Merge is a left fold over the layers, seeded with an empty object: one walk,
`merge` / `merge_all`, with no format in sight.

**Why an IR, when an earlier draft of this document forbade one.** The rule
"never as a hidden IR" existed to protect TOML datetime fidelity, and it did so
by paying for two near-identical merge walks that differed in exactly two tokens
— and it still leaked, because a mixed-format merge had to detour through
`serde_json::Value` anyway (§3.2). A `Datetime` variant protects the same
fidelity directly, in about five lines of conversion and no merge logic, and it
protects it on the mixed path too.

The two variants that are not universal are exactly the two known mismatches of
§3.2, and each is handled at the one boundary where it is impossible:
`Datetime` renders as a string in JSON, `Null` is an error in TOML.

`Number` keeps three variants because JSON integers above `i64::MAX` (snowflake
IDs, hashes) are real and `f64` rounds them silently. `Number::from_u64` demotes
to `I64` whenever the value fits, so every integer has one representation and
derived `PartialEq` does not become source-dependent.

### 2.1 Merge semantics

| Case | Behaviour |
| --- | --- |
| object ⊕ object | recurse per key |
| array ⊕ anything | **replace wholesale**, never index-merge or concat |
| scalar ⊕ anything | last wins |
| anything ⊕ null | null is an ordinary value; it overwrites |

```rust
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => { b.insert(k, v); }
                }
            }
        }
        (b, o) => *b = o,
    }
}
```

**Arrays replace.** Lodash-style index-merging turns `["a"]` over
`["x","y","z"]` into `["a","y","z"]` — a value nobody wrote. No
`--array-strategy` flag; if concatenation is ever needed it can be added, but
the default must not be surprising.

**Null is a value, not a delete.** RFC 7386 merge-patch semantics (null removes
the key) were considered and rejected:

- `knf a.json` with one argument must be a no-op. Delete semantics silently
  strips nulls from a single file, breaking the most basic invariant.
- YAML's bare `foo:` parses as null, so a typo would silently delete a key.
- Null-as-value is the reversible choice: `--null-delete` can be added later
  as opt-in. Delete-by-default cannot be taken back once trees depend on it.

**Merge is not associative.** Any scalar shadowing an object breaks it:

```
{a:{b:1}} ⊕ {a:5} ⊕ {a:{c:2}}
  left-assoc  → {a:{c:2}}
  right-assoc → {a:{b:1,c:2}}
```

Consequence: **always strictly left-fold the final flat layer list.** Never
merge subgroups and then combine them. This warrants a comment on `merge_all`.

### 2.2 `--strict`

Errors when a layer changes the *type* of an existing key (object → scalar,
int → array, anything → null). Catches the class of mistake where a leaf
accidentally shadows a subtree.

Pleasant side effect: strict mode rejects exactly the type changes that break
associativity, so under `--strict` the merge *is* associative.

Applies to `--set` layers too — shadowing a subtree is easier to do from the
shell than in a file.

### 2.3 Input constraints

Every input must deserialize to an **object at the top level**. A bare array or
string root is legal JSON/YAML but is not a config, cannot be emitted as TOML,
and produces nonsense under last-wins. One-line check, error naming the file.

---

## 3. Formats

**v1 ships JSON and TOML only.** YAML is deferred: `serde_yaml` is archived
upstream, the maintained forks (`serde_yaml_ng`, `serde_norway`) are unproven,
and YAML drags in edge cases the other formats don't have (non-string keys,
bare-key nulls, anchor expansion). Adding a format is one arm of a `match`;
removing one is a breaking change.

```rust
enum Format { Json, Toml }
```

Inferred from file extension. `--input-format` overrides (required for stdin).

### 3.1 Output format

- All inputs the same format → emit that format.
- Mixed inputs → `--format` is **required**, no guessing.

Following the first file's format would mean reordering arguments silently
changes the output encoding. `--format`/`-f` selects explicitly.

Pretty-printed by default so `knf a.json b.json > merged.json` produces a
reviewable diff. `--compact` to opt out.

### 3.2 IR↔format conversion

Four total-or-nearly-total functions in `value.rs`, free rather than trait impls
because both sides are foreign types (`impl From<toml::Value> for
knf_merge::Value` names nothing local and does not compile):

```rust
fn from_json(serde_json::Value) -> Value;
fn to_json(Value)   -> serde_json::Value;          // Datetime → String
fn from_toml(toml::Value) -> Value;                // Datetime → Datetime
fn to_toml(Value)   -> Result<toml::Value, NullInToml>;
```

Two known mismatches. The rule for which to fix and which to surface:

> **User-data impossibility → surface it. Type-boundary artefact → convert it
> explicitly, at the one boundary where the type genuinely cannot exist.**

| Issue | Kind | Action |
| --- | --- | --- |
| TOML datetime has no JSON equivalent | type mismatch | `to_json`: datetime → string |
| JSON null has no TOML equivalent | genuine impossibility in user data | `to_toml`: **error** |

**Datetimes.** `Value::Datetime` carries the source spelling, which round-trips
exactly through `Display`/`FromStr` for all four TOML forms (offset datetime,
local datetime, local date, local time, fractional seconds included). So the
stringification happens only on the way *out* to JSON: `-f json` prints a
string, and every TOML output — homogeneous, mixed, or with `--set` on top —
prints the datetime unquoted. `--strict` therefore also still sees `datetime`
vs `string` when a JSON layer overrides one.

`to_toml`'s `Datetime` arm re-parses with `.expect`, infallible by construction:
the only producers of IR values are the TOML parser, the JSON parser and `--set`
(a JSON-parsed RHS), and only the first ever emits `Datetime`.

**Nulls.** `to_toml` walks the tree for null paths before converting. serde's own
message ("unsupported None value") has no key path, and `toml`'s map serializer
*skips* `None` entries rather than failing — so a pre-check is the only way to
surface the impossibility. Provenance is a post-hoc lookup over the layers that
went into the merge (including `--set`), not threaded through the merge.

The pre-check runs on the **merged** document, not per layer, so `--set a=null
--set a=1` succeeds: the null never reaches conversion. `--set a=null` alone on
TOML output is an error like any other null, and the report names the
expression: `--> proxy   (from --set proxy=null)`.

Not listed above, because it turned out not to exist: `ValueAfterTable`, which
0.5-era `toml` raised when a scalar followed a table in the same table. Modern
`toml` buffers each table into its own body and emits them in creation order, so
scalars always land in their own parent's body. No key reordering is needed.

```
error: cannot serialize null to TOML
  --> servers.primary.proxy   (from config/override.json)
  --> logging.sink            (from config/override.json)
help: emit JSON with -f json, or remove the null
```

### 3.3 Key ordering

The IR's `Object` is an `IndexMap`, so the merge itself preserves order
unconditionally. That is necessary but **not sufficient**: both writers buffer
into their own map type on the way out, and both default to `BTreeMap`. So
**`preserve_order` must be enabled on `serde_json` *and* `toml`** — without it
the IR preserves order faithfully and the writer re-sorts it anyway.

---

## 4. CLI

One command. Exactly one document to stdout, always.

### 4.1 `knf <files...>`

```bash
knf base.toml prod.toml
knf base.toml - --input-format json          # stdin as a layer
knf base.toml --set server.port=8080 -f json
```

Files are **layers**, merged left to right in argument order. Exactly one
document to stdout, always.

**Directories are not accepted.** Flattening a directory into layers would be
silent and delayed — a single-file `db/` works, then someone adds a second file
six months later and the config quietly becomes a union of two files that were
never meant to stack. `knf config/*.toml` covers the flat case and lets the
shell do the sorting.

This leaves the command with **no argument parsing rules at all**: every
positional is a path, `-` is stdin.

### 4.2 `--set key.path=value`

A terminal layer built from the command line, appended after all files. Applies
last, always; multiple `--set` apply left to right. No new merge semantics.

A flag rather than a positional, to keep the command free of parsing rules.

RHS is parsed as JSON with a string fallback:

```
port=8080       → 8080     (number)
debug=true      → true     (bool)
name=foo        → "foo"    (JSON parse fails → string)
proxy=null      → null     (an error under -f toml, like any other null)
tags=["a","b"]  → array
tags=[a,b]      → "[a,b]"  (parse fails → string)
```

Sharp edge to document in `--help`: `version=1.0` becomes the number `1.0`.
Force a string by quoting into JSON: `--set version='"1.0"'`.

Dotted paths nest: `server.port=8080` → `{"server":{"port":8080}}`. Keys
containing literal dots are therefore not addressable — no escape syntax; use a
file.

`knf --set a.b=1` with no files is legal and emits `{"a":{"b":1}}`.

### 4.3 Flag summary

| Flag | Meaning |
| --- | --- |
| `-f, --format` | output format; required when inputs are mixed |
| `--input-format` | input format override; required for `-` |
| `--set k.p=v` | inline terminal layer, repeatable |
| `--strict` | error on type changes across layers |
| `--compact` | disable pretty-printing |

---

## 5. Package structure

A **workspace with the merge core and the dotted-path parser in their own
crates, `publish = false`.**

```
knf/
├── Cargo.toml                  [workspace] members = ["crates/*"]
├── PLAN.md
└── crates/
    ├── knf-merge/              publish = false
    │   ├── Cargo.toml          deps: thiserror, indexmap. No features.
    │   ├── src/
    │   │   ├── lib.rs          merge, merge_all, MergeOptions, MergeError
    │   │   ├── value.rs        Value, Number, Map
    │   │   └── strict.rs       type-conflict detection
    │   └── tests/
    │       ├── cases.rs        the case table
    │       ├── common/mod.rs   JSON literal → IR (dev-only)
    │       └── props.rs        proptest
    ├── knf-dotted/             publish = false
    │   ├── Cargo.toml          deps: thiserror; serde_json/serde optional
    │   └── src/
    │       ├── lib.rs          PathLeaf<V>, ParseError
    │       └── json.rs         FromStr/From for serde_json::Value  [feature = json]
    └── knf/
        ├── Cargo.toml          deps: knf-merge, knf-dotted, serde_json, toml, clap, anyhow
        ├── src/
        │   ├── main.rs         thin: parse args, call lib, map errors to exit codes
        │   ├── lib.rs          pipeline + error type
        │   ├── cli.rs          clap derive structs
        │   ├── format.rs       Format enum, parse, emit
        │   └── value.rs        IR↔JSON/TOML conversion, NullInToml
        └── tests/
            └── cli.rs
```

Rough sizes: `knf-merge` ~150 lines, `knf-dotted` one type, `value.rs` holds the
conversions, everything else small.

### 5.1 Why a separate crate, and why unpublished

The value is **compiler-enforced separation**, not distribution. A `use
clap::...` accidentally added to the merge core becomes a build error rather
than a slow leak, which is what keeps the core genuinely reusable and keeps its
tests fast and process-free.

`publish = false` because the merge semantics are the part still in flux. Two
items on the deferred list (`--null-delete`, `--array-strategy concat`) change
the public signature — `merge(a, b)` becomes `merge(a, b, &Options)`. A crate
boundary is a semver promise, and promising an API for behaviour explicitly
deferred is backwards. Inside an unpublished workspace member that change is
one commit; on crates.io it is a major bump and a coordinated release.

Flip to `publish = true` once the options struct has settled and something
other than `knf` wants to depend on it. Before doing so, check prior art —
`json-patch` implements RFC 7386, and generic deepmerge crates exist — if only
for naming.

### 5.2 Boundary rules for `knf-merge`

These are what make the separation real rather than cosmetic. Enforced by
`cargo tree -p knf-merge --depth 1` staying at `thiserror` and `indexmap`.

- **No format crate, at all.** `thiserror` for errors and `indexmap` for the
  object type — the latter format-neutral, and required because a
  `Vec<(String, Value)>` would make merge's per-key lookup quadratic. No
  `serde_json`, no `toml`, no `anyhow`, no `clap`, no `std::path`, no I/O.
  `serde_json` is a **dev**-dependency for the test literals only; dev
  dependencies do not propagate to consumers.
- **No cargo features.** There is nothing left to gate: the value type is owned,
  so the crate compiles one way. This also removes the failure mode where
  `cargo build -p knf-merge` and `cargo test --workspace` disagree.
- **Errors are a local `MergeError` enum**, never `anyhow::Error`. This is the
  rule that actually holds the line — the moment merge returns
  `anyhow::Result`, it is welded to the binary.
- **Conflicts carry a key path (`Vec<String>`) and nothing else.** No
  filenames, no file indices, no layer numbers. Provenance is the caller's job,
  which is already how the null-in-TOML error works (§3.2: post-hoc lookup over
  the layers, not threaded through the merge).
- **Strict mode is a parameter, not a build feature.** Cargo features are
  additive and unify across a dependency graph; a `strict` feature would be a
  correctness hazard the moment a second consumer exists.
- **Merge is two concrete functions, not a trait.** `merge` and `merge_all`,
  over one owned `Value`. A trait abstracting `serde_json::Value` and
  `toml::Value` was the alternative; it needs a GAT for the map borrow and buys
  nothing the IR does not.

Parse/emit stay in `knf` (`format.rs`); IR↔format conversion stays in
`value.rs`. `--set` is clap in `knf` over `knf_dotted::PathLeaf`.

### 5.3 Boundary rules for `knf-dotted`

Same idea as §5.2: compiler-enforced separation, not distribution.

- **`thiserror` is the only hard dependency.** `serde_json` and `serde` are
  optional behind `json`. No `anyhow`, no `clap`, no I/O. There is no TOML leaf:
  the `--set` RHS is JSON in both directions now, and the IR takes it from there.
- **Errors are a local `ParseError` enum**, never `anyhow::Error`, and never
  mention `--set`. Provenance is the caller's job.
- **Two conversions, not one.** `From<PathLeaf<V>> for V` expands to a nested
  object. Serde (de)serializes the expression string. Mixing those would make
  `serde_json::to_value` surprise every caller. `FromStr` exists for
  `PathLeaf<String>` (raw RHS) and `PathLeaf<serde_json::Value>`.
- **`Display` is canonical** for typed leaves (dotted path, `=`, compact JSON).
  `PathLeaf<String>` displays the raw RHS.

### 5.4 Dependencies

| Crate | Where | Why |
| --- | --- | --- |
| `indexmap` | knf-merge | the IR's object type; order-preserving, format-neutral |
| `thiserror` | knf-merge, knf-dotted | typed errors |
| `serde_json` (`preserve_order`) | knf-dotted, knf | JSON parse/emit; `--set` RHS |
| `serde` | knf-dotted | string (de)serialize for typed `PathLeaf` |
| `toml` (`preserve_order`) | knf | TOML parse/emit |
| `clap` (`derive`) | knf | CLI |
| `anyhow` | knf | error plumbing in `main` |

`preserve_order` must be enabled in the **workspace**, not just one member.
Cargo unifies features across the graph, so a member enabling it silently
changes map behaviour for every other member — better to make that explicit
than to discover it via a key-ordering test failing only under `cargo test
--workspace`. `knf-merge` no longer needs it (its `Map` is an `IndexMap`
outright), but both writers still re-sort without it.

Dev: `insta`, `proptest` (knf-merge), `assert_cmd`, `tempfile` (knf).

### 5.5 Testing shape

The crate split makes the test split fall out on its own: `knf-merge` and
`knf-dotted` tests are pure values with no filesystem and no process, so
`cargo test -p knf-merge` / `cargo test -p knf-dotted` is the fast inner loop.

In `knf-merge`:

- **Table tests + `insta` snapshots** for the bulk: `(layers, options) →
  expected`. Adding a case should be a one-liner.
- **`proptest`** for two cheap properties that catch real bugs:
  `merge(a, a) == a` and `merge(merge(a, b), b) == merge(a, b)`.

In `knf-dotted`:

- The §4.2 typing table, canonical `Display`, and the serde-as-string vs
  nested-object split.

In `knf`:

- **Round-trip tests per format**, which is where TOML datetimes, large
  unsigned integers, and the conversions of §3.2 get caught. These belong here,
  not in the core — `knf-merge` has no format crate to round-trip through.
- **`assert_cmd`** reserved for genuinely CLI-level behaviour: exit codes,
  stdin, the directory-rejected error text, the null-in-TOML error text.

---

## 6. Deferred

Listed so they aren't re-litigated mid-implementation.

- **`--explain <dotted.key>`** — which file last wrote a given path. Obvious
  next feature, but its shape depends on whether real confusion turns out to be
  about layer ordering. Wait for the first issue.
- **YAML** — one match arm, once a fork proves itself.
- **`--null-delete`** — opt-in RFC 7386 semantics, if anyone asks.
- **`--array-strategy concat`** — same.
- **TOML-native null input** (a sentinel key/value) — rejected for now: any
  sentinel is also a legal string, so failures would be silent and
  data-dependent, and the file's meaning would depend on a flag not present in
  the file. Dropping one JSON layer into the tree already solves it, which is
  half the point of being multi-format. Revisit only if someone has a genuinely
  all-TOML tree they cannot add a JSON file to.
- **Publishing `knf-merge`** — once the options struct stops moving (i.e. once
  `--null-delete` and `--array-strategy` are decided one way or the other) and
  something other than `knf` wants it. See §5.1.
- **Comment preservation** — impossible with a `Value` IR in any language.
  Would be a different tool built on `toml_edit`, single-format only.
