# Interpolation

## Document and environment references

`f.toml`:

```toml
root = "/srv"
data_dir = "${root}/data"
port = "${env:KNF_TEST_PORT}"
url = "http://localhost:${env:KNF_TEST_PORT}/health"
literal = "$${NOT_A_REF}"
```

### References pass through without the flag

```console
$ KNF_TEST_PORT=8080 knf f.toml
```

`stdout`:

```text
root = "/srv"
data_dir = "${root}/data"
port = "${env:KNF_TEST_PORT}"
url = "http://localhost:${env:KNF_TEST_PORT}/health"
literal = "$${NOT_A_REF}"
```

### Interpolate resolves documents and the environment

```console
$ KNF_TEST_PORT=8080 knf f.toml --interpolate
```

`stdout`:

```text
root = "/srv"
data_dir = "/srv/data"
port = 8080
url = "http://localhost:8080/health"
literal = "${NOT_A_REF}"
```

## Whole-string references keep the referent's type

`f.json`:

```json
{"p":8080,"db":{"host":"local"},"port":"${p}","alias":"${db}","label":"port ${p}"}
```

```console
$ knf f.json --interpolate --compact
```

`stdout`:

```text
{"p":8080,"db":{"host":"local"},"port":8080,"alias":{"host":"local"},"label":"port 8080"}
```

## Nested references select a subtree from the environment

`f.toml`:

```toml
db = "${databases.${env:KNF_TEST_STAGE}}"

[databases.dev]
host = "localhost"

[databases.prod]
host = "db.example"
```

```console
$ KNF_TEST_STAGE=prod knf f.toml --interpolate
```

`stdout`:

```text
[db]
host = "db.example"

[databases.dev]
host = "localhost"

[databases.prod]
host = "db.example"
```

## An unresolved nested reference names the looked-up path

`a.json`:

```json
{"stage":"qa","db":"${databases.${stage}}"}
```

```console
$ knf a.json -i
? 1
```

`stderr`:

```text
error: unresolved reference
  --> db: `databases.qa`
help: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable
help: drop --interpolate to pass `${...}` through untouched
```

## References read array elements

`f.json`:

```json
{"servers":[{"host":"a","port":5432}],"url":"http://${servers[0].host}:${servers[0].port}","primary":"${servers[0]}"}
```

```console
$ knf f.json --interpolate --compact
```

`stdout`:

```text
{"servers":[{"host":"a","port":5432}],"url":"http://a:5432","primary":{"host":"a","port":5432}}
```

## References read the merged document

`base.json`:

```json
{"host":"local","url":"http://${host}/"}
```

`prod.json`:

```json
{"host":"prod"}
```

```console
$ knf base.json prod.json --interpolate --compact
```

`stdout`:

```text
{"host":"prod","url":"http://prod/"}
```

## Set layers interpolate too

`f.json`:

```json
{"root":"/srv"}
```

```console
$ knf f.json -c 'data=${root}/data' --interpolate --compact
```

`stdout`:

```text
{"root":"/srv","data":"/srv/data"}
```

## A null referent remains null

`f.json`:

```json
{"n":null,"copy":"${n}"}
```

```console
$ knf f.json --interpolate --compact
```

`stdout`:

```text
{"n":null,"copy":null}
```

## A referenced table merges with a later table

`base.toml`:

```toml
[bar]
a = 1
d = 1

[foo]
b = "${bar}"
```

`over.toml`:

```toml
[foo.b]
a = 4
```

`late.toml`:

```toml
[bar]
d = 9
```

### Interpolated from the layers given

```console
$ knf base.toml over.toml -i
```

`stdout`:

```text
[bar]
a = 1
d = 1

[foo.b]
a = 4
d = 1
```

### A later layer updates the referent

```console
$ knf base.toml over.toml late.toml -i
```

`stdout`:

```text
[bar]
a = 1
d = 9

[foo.b]
a = 4
d = 9
```

### Shallow merge replaces the referenced table

```console
$ knf base.toml over.toml -i --shallow foo.b
```

`stdout`:

```text
[bar]
a = 1
d = 1

[foo.b]
a = 4
```

## Errors

### Unresolved references

`f.json`:

```json
{"server":{"url":"${db.hostname}"},"tags":["${env:KNF_TEST_REGION}"]}
```

```console
$ knf f.json --interpolate
? 1
```

`stderr`:

```text
error: unresolved reference
  --> server.url: `db.hostname`
  --> tags[0]: `env:KNF_TEST_REGION`
help: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable
help: drop --interpolate to pass `${...}` through untouched
```

### Container references embedded in a string

`f.json`:

```json
{"db":{"host":"x"},"xs":[1],"url":"http://${db}/","tag":"<${xs}>"}
```

```console
$ knf f.json --interpolate
? 1
```

`stderr`:

```text
error: reference cannot be rendered into a string
  --> url: `db` is an object
  --> tag: `xs` is an array
help: an object or array reference must be the whole string, not embedded in one
```

### Malformed references

`f.json`:

```json
{"a":"${b","c":"${missing} ${}","d":"${env:}","e":"${x..y}"}
```

```console
$ knf f.json --interpolate
? 1
```

`stderr`:

```text
error: invalid reference
  --> a: unterminated `${` at offset 0
  --> c: empty reference `${}`
  --> d: empty variable name in `${env:}`
  --> e: empty segment in reference `${x..y}`
unresolved reference
  --> c: `missing`
help: a reference is `${key.path}` (with `[n]` for array elements) or `${env:NAME}`; write `$$` for a literal `$`
help: `${key.path}` names a key in the merged document, `${env:NAME}` an environment variable
help: drop --interpolate to pass `${...}` through untouched
```

### Malformed index

`f.json`:

```json
{"tags":["a"],"t":"${tags[x]}"}
```

```console
$ knf f.json --interpolate
? 1
```

`stderr`:

```text
error: invalid reference
  --> t: malformed index in reference `${tags[x]}`
help: a reference is `${key.path}` (with `[n]` for array elements) or `${env:NAME}`; write `$$` for a literal `$`
help: drop --interpolate to pass `${...}` through untouched
```

### Reference cycle

`f.json`:

```json
{"a":"${b}","b":"${c}","c":"${a}"}
```

```console
$ knf f.json --interpolate
? 1
```

`stderr`:

```text
error: reference cycle: `a` -> `b` -> `c` -> `a`
help: a reference may not resolve, directly or indirectly, to itself
```
