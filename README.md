# knf

Merges layered configuration files and prints the result. One job, no query
language, no template engine.

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

### Override merge behavior on specific paths

What if a document has one array that should be appended to, and not replaced? 
`knf` understands how to override the merge behavior on a specifc path: 
```bash
knf base.toml prod.toml --append plugins    # concatenate, base ++ prod
knf base.toml prod.toml --replace db        # take prod's [db] whole
knf base.toml prod.toml --fail db.host      # error if prod overrides db.host
```

| Flag | At that path |
| --- | --- |
| `--append` | concatenate; both sides must be arrays |
| `--replace` | assign wholesale, no recursion, even object over object |
| `--fail` | error; the first layer to define the path pins it |

### Variable and environment references

A merged config often wants to refer to itself, or to the environment.
`--interpolate` resolves `${key.path}` and `${env:VAR}` in string values, in one
pass over the merged document:

```toml
# base.toml
root     = "/srv"
data_dir = "${root}/data"
port     = "${env:PORT}"
url      = "http://localhost:${env:PORT}/health"
literal  = "$${NOT_A_REF}"
```

```console
$ PORT=8080 knf base.toml --interpolate
root = "/srv"
data_dir = "/srv/data"
port = 8080
url = "http://localhost:8080/health"
literal = "${NOT_A_REF}"
```

**It is opt-in, and off by default.** knf sits directly upstream of tools whose
own syntax is `${...}` — compose files, GitHub Actions workflows, Helm charts,
systemd units. Eating those without being asked would be silent corruption, so
without the flag the output is byte for byte what it is today.

Where the reference sits decides what it yields:

| Position | Behaviour |
| --- | --- |
| whole string — `port = "${p}"` | takes the referent's **value and type**; `port` above is a number, and `"${db}"` is the whole table |
| embedded — `url = "x/${p}"` | stringifies; an object or array has no format-independent spelling here, so it is an error |

An environment variable is typed by the same rule as `--set`'s right-hand side
when it is the whole string, and spliced as raw text when it is embedded —
parsing it only to print it again could only lose something.

`$$` is a literal `$`. A `$` followed by anything else is ordinary text, so
`USD $5` needs no escaping.

Document references resolve transitively and in any order; environment values
are terminal and are never re-scanned. Cycles are an error, and so is a
reference that names nothing:

```
$ knf base.toml --interpolate
error: unresolved reference
  --> server.url: `db.hostname`
  --> tags[0]: `env:REGION`
help: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable
help: drop --interpolate to pass `${...}` through untouched
```

A reference may also read an array element — `${servers[0].host}` — with the
same two-position rules: whole-string it takes the element's value and type,
embedded it stringifies.

Two limits worth knowing:

- **`env:` is a reserved prefix**, matched literally rather than by splitting on
  the first `:`. So `${a:b}` is the ordinary key `a:b`, and only keys that
  literally begin `env:` are unaddressable.
- **A key spelled with brackets is unaddressable** — `${a[0]}` now reads as *the
  first element of `a`*, never as a key literally named `a[0]`, even though a
  file or `--set 'a[0]=1'` can still write one. The same accepted loss as keys
  containing a literal dot, which the dotted grammars have always excluded.

`--set` layers interpolate like any other layer. `--strict` runs during the
merge, before any substitution, so it compares the types values had when they
were written.

## Caveats with formats

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
an error that names every path:

```
$ knf base.toml override.json -f toml
error: cannot serialize null to TOML
  --> servers.primary.proxy
  --> logging.sink
help: emit JSON with -f json, substitute with --null-as, or remove the null
```

Alternatively, you may use `--null-as <string>` to parse nulls into a custom value:

```bash
knf base.toml override.json -f toml --null-as=none
```
The option is a no-op for JSON output. 

## Testing

```bash
cargo test --workspace
cargo test -p knf-core           # fast inner loop: no filesystem, no process
```

## License

MIT — see [LICENSE](LICENSE).
