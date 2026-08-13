# knf

Merges layered configuration files and prints the result. One job, no query
language, no templating.

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json --set server.port=8080
knf matrix config/ --out-dir out/
```

It exists because the alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

Two capabilities beyond plain merging:

- **Cross-format merging.** JSON and TOML layers mix freely, because everything
  is parsed into one in-memory representation.
- **Composable config trees** (`knf matrix`). A directory tree describes a set
  of variants; `knf` enumerates and materialises them, so downstream consumers
  read a single flat config instead of resolving layers themselves.

## Install

```bash
cargo install --path crates/knf
```

## Merging

Files are layers, merged left to right in argument order. Exactly one document
goes to stdout.

| Case | Behaviour |
| --- | --- |
| object ⊕ object | recurse per key |
| array ⊕ anything | **replace wholesale**, never index-merge or concat |
| scalar ⊕ anything | last wins |
| anything ⊕ null | null is an ordinary value; it overwrites |

Two consequences worth knowing:

- **Arrays replace.** Index-merging would turn `["a"]` over `["x","y","z"]` into
  `["a","y","z"]` — a value nobody wrote.
- **Null is a value, not a delete.** So `knf a.json` with one argument is always
  a byte-level no-op.

`--strict` errors when a layer changes the *type* of an existing key, which
catches the class of mistake where a leaf accidentally shadows a subtree.

```
$ knf a.json b.json --strict
error: type conflict at `server`: object would be replaced by number
```

### Formats

JSON and TOML, inferred from the file extension. `--input-format` overrides it
for every input and is required for `-` (stdin).

Output is the inputs' format when they agree; when they don't, `-f` is required
rather than guessed, so reordering arguments can never silently change the
encoding. Pretty-printed by default; `--compact` opts out.

TOML cannot represent null, so emitting a document containing one is an error
that names the paths and the file each came from:

```
$ knf base.toml override.json -f toml
error: cannot serialize null to TOML
  --> servers.primary.proxy   (from override.json)
  --> logging.sink            (from override.json)
help: emit JSON with -f json, or remove the null
```

### `--set`

A terminal layer built from the command line, applied after all files. The value
is parsed as JSON with a string fallback:

```
port=8080       → 8080     (number)
debug=true      → true     (bool)
name=foo        → "foo"    (JSON parse fails → string)
proxy=null      → null     (a value, not a delete)
tags=["a","b"]  → array
tags=[a,b]      → "[a,b]"  (JSON parse fails → string)
```

Sharp edge: `version=1.0` is the *number* `1.0`. Force a string by quoting into
JSON: `--set version='"1.0"'`.

Dotted paths nest, so keys containing a literal dot are not addressable from
`--set` — use a file.

## `knf matrix`

A **directory is a group; the files within it are mutually exclusive
alternatives.** Each subdirectory is an independent axis that also applies.

```
config/
  base.toml            → singleton group, auto-selected
  db/
    mysql.toml         → group 'db', 2 alternatives
    postgres.toml
  server/
    apache.toml        → group 'server', 2 alternatives
    nginx.toml
```

→ 2 × 2 = 4 documents, each merging `base` + one db + one server.

Grouping is keyed by parent directory, not by depth. Pooling by depth would turn
`db/` and `server/` into one four-way axis, yielding configs with a db *or* a
server and never both.

Singleton groups auto-select; multi-file groups must be pinned, or every
combination is produced:

```bash
knf matrix config/ db=postgres server=nginx      # → one document, stdout
knf matrix config/ db=postgres --out-dir out/    # → 2 documents (server axis)
knf matrix config/ --out-dir out/                # → 4 documents
knf matrix config/ --list                        # → the combinations, no writes
knf matrix config/                               # → error:
```

```
error: ambiguous group `db` — 2 alternatives
  db=mysql, db=postgres
ambiguous group `server` — 2 alternatives
  server=apache, server=nginx
help: knf matrix config/ db=mysql server=apache
help: or write every combination — knf matrix config/ --out-dir out/
```

Because every group in a one-file-per-directory tree is a singleton, such a tree
resolves with no pinning and behaves exactly like a plain layered merge. Groups
are invisible until someone creates one.

Output files are named by choice, so a filename is reversible and unambiguous
even when two groups share a choice name:

```
out/db=postgres,server=nginx.toml
out/db=postgres/server=nginx.toml     # --tree
```

`--separator` overrides the joiner between pairs (the `=` is fixed), and `--max`
(default 256) caps the product size before anything is written, mainly to catch
`matrix` pointed at the wrong directory.

Arguments are split purely syntactically: one containing `=` is a group pin,
anything else is the directory. Consequence: a file named exactly `matrix` in
the current directory parses as the subcommand — write `./matrix`.

### Walker rules

- Extension allowlist (`.json`, `.toml`); everything else is skipped silently, so
  a `README.md` in a config directory is not an error.
- Dotfiles and dot-directories are skipped, so `knf matrix .` does not walk
  `.git`.
- Symlinks are not followed.
- A directory contributing neither files nor a contributing subdirectory is an
  error naming that directory.
- Byte-wise lexicographic sort within a directory — **no natural/numeric sort**,
  so `10-x` precedes `2-x`.

## Layout

```
crates/
  knf-merge/    the merge core: serde_json + thiserror, and nothing else
  knf/          CLI, formats, matrix
```

The core is a separate crate for compiler-enforced separation, not for
distribution — a `use clap::...` added to it becomes a build error rather than a
slow leak. It is `publish = false` while the merge semantics are still moving.

```bash
cargo test --workspace
cargo test -p knf-merge          # fast inner loop: no filesystem, no process
```

See `PLAN.md` for the design and the reasoning behind each decision.
