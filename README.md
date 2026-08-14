# knf

Merges layered configuration files and prints the result. One job, no query
language, no templating.

```bash
# Print the output to stdout
knf base.toml prod.toml > merged.toml
# Add manual overrides via the --set flag
knf defaults.json overrides.json --set server.port=8080 --set host=name
# Mix toml and json (if you want)
knf *.toml *.json
```

It exists because more powerful alternatives (`yq ea '. as $i ireduce ({}; . * $i)'`,
`jq -s 'reduce ...'`) require non-obvious incantations for what is a common,
simple operation. `knf <files>` should need no explanation.

## Installation

```bash
cargo install knf
```

## Merging

Files are merged left to right in argument order. Exactly one document
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

### Caveats with formats

JSON and TOML, inferred from the file extension. `--input-format` overrides it
for every input and is required for `-` (stdin).

Output is the inputs' format when they agree; when they don't, `-f` is required
rather than guessed, so reordering arguments can never silently change the
encoding. Pretty-printed by default; `--compact` opts out.

A TOML datetime is a distinct type all the way through the merge, so every TOML
output keeps it unquoted — including a merge that mixed in a JSON layer, and
including `--set` on top. It becomes a plain string only under `-f json`, where
there is nothing else it could be.

TOML cannot represent null, so emitting TOML from a document containing one is
an error that names the paths and where each came from:

```
$ knf base.toml override.json -f toml
error: cannot serialize null to TOML
  --> servers.primary.proxy   (from override.json)
  --> logging.sink            (from override.json)
help: emit JSON with -f json, or remove the null
```

## Testing

```bash
cargo test --workspace
cargo test -p knf-merge          # fast inner loop: no filesystem, no process
```

## License

MIT — see [LICENSE](LICENSE).
