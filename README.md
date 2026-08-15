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
cargo install knf-cli
```

## Library

```bash
cargo add knf-core
```

```rust
use knf_core::{Value, merge};

let merged = merge([base, overlay])?;
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

- **Arrays replace**, unless `--append` names the path. Index-merging would turn
  `["a"]` over `["x","y","z"]` into `["a","y","z"]` — a value nobody wrote.
- **Null is a value, not a delete.** So `knf a.json` with one argument is always
  a byte-level no-op.

`--strict` errors when a layer changes the *type* of an existing key, which
catches the class of mistake where a leaf accidentally shadows a subtree.

```
$ knf a.json b.json --strict
error: type conflict at `server`: object would be replaced by number
```

### Per-path strategies

Most documents have one array that should accumulate and another that should be
overridden, so the choice is per key, not per run. Three repeatable flags name a
path and change what happens *there*:

```bash
knf base.toml prod.toml --append plugins    # concatenate, base ++ prod
knf base.toml prod.toml --replace db        # take prod's [db] whole
knf base.toml prod.toml --fail db.host      # error if prod sets it again
```

| Flag | At that path |
| --- | --- |
| `--append` | concatenate; both sides must be arrays |
| `--replace` | assign wholesale, no recursion, even object over object |
| `--fail` | error; the first layer to define the path pins it |

The rules live on the command line rather than in the data, deliberately. In-data
markers (`plugins+ = [...]`, Kubernetes' `$patch: replace`) mean a layer is no
longer a file your app, editor and linter can read — and that portability is the
point of knf. Rules stay in argv; layers stay plain config.

Some details that follow from that:

- **Rules are a set, so flag order never affects the output.** A path named by two
  or more of the flags is an error, not a race — and one error naming every flag
  involved, not one per pair.
- **Nothing may nest under a rule.** All three consume the whole value at their
  path, so `--replace db --append db.plugins` is rejected — before any file is
  read, since it is a mistake in the command line alone.
- **A path with no existing value is inserted, whatever the rule says.** That is
  what keeps `--fail` meaning "the first layer to set this pins it" rather than
  "this key may never exist", and what keeps `--append` from doubling a lone
  layer's array.
- **`--set` is an ordinary layer**, so rules apply to it too: `--replace db --set
  db.host=x` leaves `db` with nothing but `host`.
- **Paths are dotted**, so a key containing a literal dot cannot be named.
- `--strict` is orthogonal, and still kind-checks the assignment `--replace` makes.

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
cargo test -p knf-core           # fast inner loop: no filesystem, no process
```

## License

MIT — see [LICENSE](LICENSE).
