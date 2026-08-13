# knf — config merge tool

**Status:** design, pre-implementation.

## 1. Idea

`knf` merges layered configuration files and prints the result. One job, no
query language, no templating.

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json --set server.port=8080
knf matrix config/ --out-dir out/
```

It exists because the alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

Two capabilities beyond plain merging:

- **Cross-format merging.** JSON and TOML layers can be mixed freely, because
  everything is parsed into one in-memory representation.
- **Composable config trees** (`knf matrix`). A directory tree describes a set
  of variants; `knf` enumerates and materialises them, so downstream consumers
  read a single flat config instead of resolving layers themselves.

---

## 2. Core model

All formats deserialize directly into `serde_json::Value`. Serde is
format-agnostic on both sides, so no conversion layer is needed:

```rust
let v: serde_json::Value = toml::from_str(&s)?;   // works as-is
```

Merging is a left fold over the layers, seeded with an empty object.

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
merge subgroups and then combine them, however tempting that looks when
implementing `matrix`. This warrants a comment in `merge_all`.

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

### 3.2 TOML-specific handling

Two known artefacts. The rule for which to fix and which to surface:

> **User-data impossibility → surface it. Tool-representation artefact → fix it
> before the user sees it.**

| Issue | Kind | Action |
| --- | --- | --- |
| Datetimes deserialize as `{"$__toml_private_datetime": "..."}` | artefact — silently produces garbage JSON | convert at **emit** time only |
| Null present when emitting TOML | genuine impossibility in user data | **error**, from a pre-check |

**Datetimes.** Parsing is unavoidable and needs no fix: `toml`'s deserializer
calls `visit_map(DatetimeDeserializer::new(v))` under `deserialize_any`, so
TOML → `serde_json::Value` yields the sentinel map. The sentinel then flows
through the merge as an ordinary object, which is correct by construction — one
key, so last-wins does the right thing.

It cannot be converted back by re-inserting the sentinel key, because the
serializer detects a datetime by **struct name** (`toml_datetime::ser::is_datetime`
on the name passed to `serialize_struct`), and a `serde_json::Map` always
serialises through `serialize_map`. So emission wraps the tree in a
`TomlValue<'a>(&'a Value)` that recognises the sentinel shape and delegates to
`Datetime::serialize`, whose own impl uses the magic struct name. That wrapper is
what makes TOML → TOML lossless. JSON output takes a cheaper route: a pre-pass
replacing sentinels with their plain string.

`$__toml_private_datetime` is `pub(crate)` in `toml_datetime` 1.x, so the literal
is hardcoded.

**Nulls.** The null case needs no flag and no silent dropping. It only triggers
when nulls survive into the final tree *and* the output is TOML — a narrow
corner. Letting serde surface it is not an option: `toml`'s map serializer
*catches* `UnsupportedNone` from a value and skips the key, which is how `Option`
struct fields work, so relying on the serializer would silently drop nulls rather
than fail. And serde's message ("unsupported None value") has no key path anyway,
leaving the user to bisect a merged tree by hand.

So: **pre-check.** Walk the merged tree for null paths before serialising, then
look each path up in the already-parsed input `Value`s and report the last file
containing it. Deterministic, independent of error text, and a post-hoc lookup
over data still in memory — no provenance threading through the merge.

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

`serde_json`'s default `Map` is a `BTreeMap`, which alphabetises output. Enable
the **`preserve_order`** feature (backs it with `indexmap`) so input ordering
survives.

That is necessary but **not sufficient**: `toml`'s document serializer buffers
into its own map type on the way out, which is also a `BTreeMap` by default. So
`preserve_order` must be enabled on **`toml` as well as `serde_json`** — without
it the IR preserves order faithfully and the writer re-sorts it anyway.

---

## 4. CLI

Two commands. Output shape is determined by the command, never by the data.

### 4.1 Default: `knf <files...>`

```bash
knf base.toml prod.toml
knf base.toml - --input-format json          # stdin as a layer
knf base.toml --set server.port=8080 -f json
```

Files are **layers**, merged left to right in argument order. Exactly one
document to stdout, always.

**Directories are not accepted here.** Under `matrix`, a directory means
"saturating paths of alternatives"; if the default command flattened the same directory
into layers, one path would mean two incompatible things. The failure would be
silent and delayed — a single-file `db/` works, then someone adds a second file
six months later and the config quietly becomes a union of two mutually
exclusive variants. `knf config/*.toml` covers the flat case and lets the shell
do the sorting.

This leaves the default command with **no argument parsing rules at all**:
every positional is a path, `-` is stdin.

### 4.2 `--set key.path=value`

A terminal layer built from the command line, appended after all files. Applies
last, always; multiple `--set` apply left to right. No new merge semantics.

A flag rather than a positional, to keep the default command free of parsing
rules.

RHS is parsed as JSON with a string fallback:

```
port=8080       → 8080     (number)
debug=true      → true     (bool)
name=foo        → "foo"    (JSON parse fails → string)
proxy=null      → null     (a value — see §2.1)
tags=["a","b"]  → array
tags=[a,b]      → "[a,b]"  (parse fails → string)
```

Sharp edge to document in `--help`: `version=1.0` becomes the number `1.0`.
Force a string by quoting into JSON: `--set version='"1.0"'`.

Dotted paths nest: `server.port=8080` → `{"server":{"port":8080}}`. Keys
containing literal dots are therefore not addressable — no escape syntax; use a
file.

`knf --set a.b=1` with no files is legal and emits `{"a":{"b":1}}`.

### 4.3 `knf matrix <dir>`

**Files in a directory are mutually exclusive layers for that node.
Subdirectories are alternative branches:** a path picks one child and continues,
and files on the way down always apply (saturation). Sibling directories are
OR, not independent axes.

```
config/
  base.toml            → prefix on every path through config/
  db/
    mysql.toml         → leaves under db/
    postgres.toml
  server/
    apache.toml        → leaves under server/
    nginx.toml
```

→ 4 documents: `base` + one db, **or** `base` + one server. Never both a db and
a server in the same document. Nested children along one lineage still apply
together: `db/{mysql,postgres}` plus `db/tuning/{small,large}` is four paths,
each with a db file and a tuning file.

Accepted cost: files in one directory are alternatives, so "apply both `a.toml`
and `b.toml` from this directory" is inexpressible. Workaround is one file per
directory. Not worth new syntax.

**Resolution.** One matching path goes to stdout. Several paths need `--out-dir`
or `--list`. `--glob` (repeatable, union) keeps only matching leaves; ancestor
files still apply. `--max-depth` (root is 0) treats a directory as a leaf even
if it has children.

```bash
knf matrix config/ --glob 'db/postgres.toml'     # → one document, stdout
knf matrix config/ --glob 'db/**' --out-dir out/ # → 2 documents (the db branch)
knf matrix config/ --out-dir out/                # → 4 documents
knf matrix config/ --list                        # → the paths, no writes
knf matrix config/                               # → error, see below
```

```
error: 4 matching paths
  db/mysql.toml
  db/postgres.toml
  server/apache.toml
  server/nginx.toml
help: knf matrix config/ --glob 'db/mysql.toml'
help: or write every path — knf matrix config/ --out-dir out/
```

Because a one-file-per-directory tree is a single path, it resolves with no
flags and behaves exactly like a plain layered merge.

No `default.toml` convention marking a fallback choice — same objection as any
in-band sentinel, and the error message already tells the user what to type.

**Output naming.** Each file is named after the leaf's path relative to the
walked directory, with the output-format extension:

```
out/db/postgres.toml
out/server/nginx.toml
```

`--separator` flattens slashes (`out/db,postgres.toml`). `--max` (default 256)
caps the number of paths before anything is written, mainly to catch `matrix`
pointed at the wrong directory. `--list` prints the resolved paths and exits
without writing.

A file named exactly `matrix` in the current directory still parses as the
subcommand — write `./matrix`.

### 4.4 Walker rules

- Extension allowlist (`.json`, `.toml`); everything else skipped silently — a
  `README.md` in a config dir is not an error.
- Skip dotfiles and dot-directories, so `knf matrix .` doesn't walk `.git`.
- Do not follow symlinks.
- Empty directory → error, not an empty object. A directory skipped by `--glob`
  or cut by `--max-depth` is not empty; it was never entered.
- `walkdir`, not `ignore`. Gitignore semantics would mean "my config was
  skipped because of a `.gitignore` three levels up" — surprising in a merge
  tool.
- Byte-wise lexicographic sort within a directory. **No natural/numeric sort** —
  it looks friendly and then generates a support question the first time
  someone has both `2-x` and `10-x`.

### 4.5 Flag summary

| Flag | Command | Meaning |
| --- | --- | --- |
| `-f, --format` | both | output format; required when inputs are mixed |
| `--input-format` | default | input format override; required for `-` |
| `--set k.p=v` | default | inline terminal layer, repeatable |
| `--strict` | both | error on type changes across layers |
| `--compact` | both | disable pretty-printing |
| `--out-dir` | matrix | write M documents here (no short flag) |
| `--glob` | matrix | keep matching leaves only; repeatable union |
| `--max-depth` | matrix | cap walk depth; root is 0 |
| `--separator` | matrix | flatten `/` in output names |
| `--list` | matrix | print resolved paths, write nothing |
| `--max` | matrix | cap on number of paths |

---

## 5. Package structure

A **workspace with the merge core in its own crate, `publish = false`.**

```
knf/
├── Cargo.toml                  [workspace] members = ["crates/*"]
├── PLAN.md
└── crates/
    ├── knf-merge/              publish = false
    │   ├── Cargo.toml          deps: serde_json, thiserror   ← and nothing else
    │   ├── src/
    │   │   ├── lib.rs          merge, merge_all, MergeError
    │   │   └── strict.rs       type-conflict detection
    │   └── tests/
    │       ├── cases.rs        table tests + insta snapshots
    │       └── props.rs        proptest
    ├── knf-fs/                publish = false
    │   ├── Cargo.toml          deps: thiserror, walkdir   ← and nothing else
    │   ├── src/
    │   │   └── lib.rs          Dir/File tree, discovery, saturating DFS paths
    │   └── tests/
    │       └── matrix.rs       enumeration table tests + walker fixtures
    └── knf/
        ├── Cargo.toml          deps: knf-merge (path), knf-fs (path), toml, clap, anyhow, globset
        ├── src/
        │   ├── main.rs         thin: parse args, call lib, map errors to exit codes
        │   ├── lib.rs          pipeline + error type
        │   ├── cli.rs          clap derive structs
        │   ├── format.rs       Format enum, parse, serialize, TOML normalize
        │   ├── set.rs          --set pair → Value
        │   └── matrix.rs       command layer: parse → merge → format → write
        └── tests/
            └── cli.rs
```

Rough sizes: `knf-merge` ~100 lines, `knf-fs` ~200, `format.rs` ~150 (most of it
TOML normalization), `matrix.rs` ~120 (command layer only), everything else
small.

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
`cargo tree` staying at two dependencies.

- **Dependencies are `serde_json` and `thiserror`. Nothing else.** No `anyhow`,
  no `clap`, no `walkdir`, no `std::path`, no I/O of any kind.
- **Errors are a local `MergeError` enum**, never `anyhow::Error`. This is the
  rule that actually holds the line — the moment merge returns
  `anyhow::Result`, it is welded to the binary.
- **Conflicts carry a key path (`Vec<String>`) and nothing else.** No
  filenames, no file indices, no layer numbers. Provenance is the caller's job,
  which is already how the null-in-TOML error works (§3.2: post-hoc lookup over
  the parsed inputs, not threaded through the merge).
- **Strict mode is a parameter, not a build feature.** Cargo features are
  additive and unify across a dependency graph; a `strict` feature would be a
  correctness hazard the moment a second consumer exists.

Everything format-specific stays in `knf/format.rs` — the TOML datetime and
`ValueAfterTable` handling of §3.2 is serialization, not merging, and pulling
it down would drag `toml` into the core.

### 5.3 Dependencies

| Crate | Where | Why |
| --- | --- | --- |
| `serde_json` (`preserve_order`) | knf-merge, knf | the IR; indexmap-backed maps |
| `thiserror` | knf-merge, knf-fs | typed errors |
| `toml` | knf | TOML in/out |
| `clap` (`derive`) | knf | CLI |
| `anyhow` | knf | error plumbing in `main` |
| `walkdir` | knf-fs | `discover` directory traversal |
| `globset` | knf | `--glob` matching for `matrix` |

`preserve_order` must be enabled in the **workspace**, not just one member.
Cargo unifies features across the graph, so a member enabling it silently
changes map behaviour for every other member — better to make that explicit
than to discover it via a key-ordering test failing only under `cargo test
--workspace`.

Dev: `insta`, `proptest` (knf-merge), `assert_cmd`, `tempfile` (knf, knf-fs).

### 5.4 Testing shape

The crate split makes the test split fall out on its own: `knf-merge` tests are
pure `Value → Value` with no filesystem and no process, so `cargo test -p
knf-merge` is the fast inner loop.

In `knf-merge`:

- **Table tests + `insta` snapshots** for the bulk: `(layers, options) →
  expected`. Adding a case should be a one-liner.
- **`proptest`** for two cheap properties that catch real bugs:
  `merge(a, a) == a` and `merge(merge(a, b), b) == merge(a, b)`.

In `knf-fs`:

- **Path enumeration is pure** — `Dir` tree → expected path list, plain table
  test, no filesystem.
- **Filesystem fixtures only for the walker**: dotfile skipping, extension
  filtering, empty-dir error, symlink handling, glob prune, max_depth.

In `knf`:

- **Round-trip tests per format**, which is where the TOML datetime and
  ordering artefacts of §3.2 get caught. These belong here, not in the core.
- **`assert_cmd`** reserved for genuinely CLI-level behaviour: exit codes,
  stdin, the ambiguous-paths error text, the null-in-TOML error text.

---

## 6. Deferred

Listed so they aren't re-litigated mid-implementation.

- **`--explain <dotted.key>`** — which file last wrote a given path. Obvious
  next feature, but its shape depends on whether real confusion turns out to be
  about layer ordering or about path selection. Wait for the first issue.
  `--list` covers most of the need meanwhile.
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
- **Publishing `knf-fs`** — extracted from `matrix.rs` into its own crate (same
  rationale as `knf-merge`: compiler-enforced separation). Unpublished because
  the path model is the least settled design in the document. Flip to
  `publish = true` once the shape stops moving and something other than `knf`
  wants it.
- **Comment preservation** — impossible with a `Value` IR in any language.
  Would be a different tool built on `toml_edit`, single-format only.
