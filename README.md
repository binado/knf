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

Files are merged left to right in argument order. In ordinary mode, exactly one
document goes to stdout.

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

## Cartesian products

A positional `x` separates factors whose files are alternatives. The shell
expands the globs, then `knf` selects one file from each factor and merges each
combination left to right:

```bash
knf foo*.toml x bar*.toml --output-dir generated
```

Given `foo1.toml`, `foo2.toml`, `bar1.toml`, and `bar2.toml`, this writes:

```text
generated/foo1+bar1.toml
generated/foo1+bar2.toml
generated/foo2+bar1.toml
generated/foo2+bar2.toml
```

The paths are also printed to stdout, one per line. Add more `x` separators for
more factors:

```bash
knf env/*.toml x region/*.toml x service/*.toml -o generated
```

In product mode, repeated `--set` expressions are alternatives in one final
factor rather than sequential layers:

```bash
knf foo*.toml x bar*.toml \
  --set region=us --set region=eu \
  -o generated
```

This produces one `region=us` and one `region=eu` variant of every file
combination, named like `foo1+bar1+region=us.toml`.

The output directory may already exist, but existing output files are never
overwritten. Factor labels are used as-is in the generated file name (so
non-ASCII text like `café.toml` passes through unchanged); a `--set`
expression containing `/`, `\`, or a NUL byte is rejected rather than
encoded. `--output-separator` overrides the default `+` delimiter used
between factor names in output files (e.g. `--output-separator _`); an empty
separator is rejected. A file literally named `x` can be passed as `./x`.

All candidates share one output format. As in ordinary mode, mixed JSON and
TOML inputs therefore require `-f json` or `-f toml`.

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
