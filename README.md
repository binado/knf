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

**Files in a directory are mutually exclusive layers for that node.
Subdirectories are alternative branches:** a path picks one child and continues,
and files on the way down always apply.

```
config/
  base.toml            → prefix on every path
  db/
    mysql.toml
    postgres.toml
  server/
    apache.toml
    nginx.toml
```

→ 4 documents: `base` + one db, **or** `base` + one server. Nested children
along one lineage still apply together.

One matching path goes to stdout. Several paths need `--out-dir` or `--list`.
`--glob` keeps matching leaves (ancestors still apply); `--max-depth` (root is
0) stops the walk early.

```bash
knf matrix config/ --glob 'db/postgres.toml'     # → one document, stdout
knf matrix config/ --glob 'db/**' --out-dir out/ # → 2 documents
knf matrix config/ --out-dir out/                # → 4 documents
knf matrix config/ --list                        # → the paths, no writes
knf matrix config/                               # → error:
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

A one-file-per-directory tree is a single path and behaves like a plain layered
merge.

Output files are named after the leaf path:

```
out/db/postgres.toml
out/server/nginx.toml
out/db,postgres.toml          # --separator ,
```

`--max` (default 256) caps the number of paths before anything is written,
mainly to catch `matrix` pointed at the wrong directory.

A file named exactly `matrix` in the current directory parses as the
subcommand — write `./matrix`.

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
  knf-fs/       directory walk and saturating DFS paths
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
