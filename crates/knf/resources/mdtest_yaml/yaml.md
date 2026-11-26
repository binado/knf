# YAML output

## JSON configs merge and interpolate in insertion order

`base.json`:

```json
{"z":{"ports":[80],"unset":null},"copy":"${z.ports}","<<":"true","strings":["null","false","12","1e3","2024-01-01"]}
```

`prod.json`:

```json
{"z":{"ports":[443,null]},"a":true}
```

```console
$ knf base.json prod.json -i --to yaml
```

`stdout`:

```text
z:
  ports:
  - 443
  - null
  unset: null
copy:
- 443
- null
"<<": "true"
strings:
- "null"
- "false"
- "12"
- "1e3"
- "2024-01-01"
a: true
```

## TOML configs merge and interpolate before conversion

`base.toml`:

```toml
[z]
ports = [80]
copy = '${z.ports}'
```

`prod.toml`:

```toml
[z]
ports = [443]
```

```console
$ knf base.toml prod.toml -i -c "a={host='local'}" --to yaml
```

`stdout`:

```text
z:
  ports:
  - 443
  copy:
  - 443
a:
  host: local
```

## Inline values use JSON rules without files or context

```console
$ knf --to yaml -c 'z={"items":[true,null]}' -c a=inf -c 'literal="42"'
```

`stdout`:

```text
z:
  items:
  - true
  - null
a: "inf"
literal: "42"
```

## Empty mapping

```console
$ knf --to yaml
```

`stdout`:

```text
{}
```

## Context selects TOML inline rules without other files

`context.toml`:

```toml
host = 'local'
```

```console
$ knf -i --with context.toml -c "db={host='\${host}'}" -c value=null --to yaml
```

`stdout`:

```text
db:
  host: local
value: "null"
```

## TOML conversion

`lossy.toml`:

```toml
[nested]
day = 1979-05-27
time = 07:32:00
noon = 12:30:00
stamp = 1979-05-27T07:32:00Z
limits = [nan, inf, -inf]
```

### Lossy values become strings

```console
$ knf lossy.toml --to yaml
```

`stdout`:

```text
nested:
  day: "1979-05-27"
  time: 07:32:00
  noon: "12:30:00"
  stamp: "1979-05-27T07:32:00Z"
  limits:
  - .nan
  - .inf
  - -.inf
```

`stderr`:

```text
warning: conversion was lossy; rerun with --strict to find the affected keys
```

### Strict names the affected key

```console
$ knf lossy.toml --to yaml --strict
? 1
```

`stderr`:

```text
error: nested.day: YAML output writes TOML dates, times and datetimes as strings
```

### Non-finite floats are native and lossless

`floats.toml`:

```toml
limits = [nan, inf, -inf]
```

```console
$ knf floats.toml --to yaml --strict
```

`stdout`:

```text
limits:
- .nan
- .inf
- -.inf
```

## Compact YAML is a usage error before reading files

```console
$ knf missing.json --to yaml --compact
? 2
```

`stderr`:

```text
error: --to yaml cannot be combined with --compact

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

## YAML snapshots are a usage error before reading files

```console
$ knf missing.json --to yaml --exec -- echo launched
? 2
```

`stderr`:

```text
error: --to yaml cannot be combined with --exec

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

## YAML input

`config.yaml`:

```yaml
port: 443
```

### YAML configs are rejected

```console
$ knf config.yaml --to yaml
? 1
```

`stderr`:

```text
error: cannot infer a format from `config.yaml`
help: name the file with a .json or .toml extension
```

### YAML context is rejected

```console
$ knf -i --with config.yaml --to yaml
? 1
```

`stderr`:

```text
error: cannot infer a format from `config.yaml`
help: name the file with a .json or .toml extension
```

## Strings that YAML readers could misread stay strings

`tricky.json`:

```json
{"y":"yes","off":"off","tilde":"~","hex":"0x1F","empty":"","padded":" a ","colon":"a: b","dash":"-","multi":"one\ntwo","é":"☃","null":1,"true":2,"e":[],"o":{},"seq":[{"k":1},[2]],"big":18446744073709551615,"f":1.5}
```

```console
$ knf tricky.json --to yaml
```

`stdout`:

```text
"y": "yes"
"off": "off"
tilde: "~"
hex: "0x1F"
empty: ""
padded: " a "
colon: "a: b"
dash: '-'
multi: |-
  one
  two
é: ☃
"null": 1
"true": 2
e: []
o: {}
seq:
- k: 1
- - 2
big: 18446744073709551615
f: 1.5
```

## Dry run ignores the output format

```console
$ knf tricky.json --dry-run --to yaml
```

`stdout`:

```text
[dry run] Skipped merging configs in this order (lowest to highest precedence):
1. tricky.json
```
