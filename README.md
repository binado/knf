# knf

Merges layered configuration files and prints the result. One job, no query
language, no templating.

```bash
knf base.toml prod.toml > merged.toml
knf defaults.json overrides.json --set server.port=8080
```

It exists because the alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

JSON and TOML layers mix freely. Each format parses into its native value type;
conversion runs only when a layer's type is not the output format.

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

A TOML datetime becomes a JSON string under `-f json`, and also when a TOML
file is mixed with a JSON file and emitted back as TOML. Homogeneous TOML
(including `--set` on TOML files) keeps datetimes unquoted.

TOML cannot represent null, so converting a document containing one is an error
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

Directories are not accepted as layers. Expand them in the shell:

```bash
knf config/*.toml
```

## Layout

```
crates/
  knf-merge/    the merge core: serde_json + thiserror, and nothing else
  knf/          CLI and formats
```

The core is a separate crate for compiler-enforced separation, not for
distribution — a `use clap::...` added to it becomes a build error rather than a
slow leak. It is `publish = false` while the merge semantics are still moving.

```bash
cargo test --workspace
cargo test -p knf-merge          # fast inner loop: no filesystem, no process
```

See `PLAN.md` for the design and the reasoning behind each decision.
