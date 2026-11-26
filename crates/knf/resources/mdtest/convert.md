# Convert

## Output format

### TOML to JSON after merging and interpolation

`base.toml`:

```toml
[db]
ports=[80, 81]
started=2024-01-01
copy='${db.started}'
```

`prod.toml`:

```toml
[db]
ports=[443]
```

```console
$ knf base.toml prod.toml -i -c db.started=2024-01-02 --to json --compact
```

`stdout`:

```text
{"db":{"ports":[443],"started":"2024-01-02","copy":"2024-01-02"}}
```

`stderr`:

```text
warning: conversion was lossy; rerun with --strict to find the affected keys
```

### JSON to TOML after interpolation

`base.json`:

```json
{"db":{"ports":[80,81],"id":null,"copy":"${db.id}"}}
```

`prod.json`:

```json
{"db":{"ports":[443],"id":42}}
```

```console
$ knf base.json prod.json -i --to toml --strict
```

`stdout`:

```text
[db]
ports = [443]
id = 42
copy = 42
```

### A format is required without input files

```console
$ knf
? 1
```

`stderr`:

```text
error: no input files to infer a format from
help: pass --to json or --to toml
```

### Inline configs alone need `--to`

```console
$ knf -c a=1
? 1
```

`stderr`:

```text
error: no input files to infer a format from
help: pass --to json or --to toml
```

### An empty JSON document

```console
$ knf --to json --compact
```

`stdout`:

```text
{}
```

### An empty TOML document

```console
$ knf --to toml
```

`stdout`:

```text

```

### Inline configs convert without input files

```console
$ knf --to toml -c a=1
```

`stdout`:

```text
a = 1
```

### Unmatched globs and missing files still allow `--to`

```console
$ knf missing.json -g '*.toml' --to toml -c a=1
```

`stdout`:

```text
a = 1
```

## Lossy conversion

### TOML datetimes become strings in JSON

`lossy.toml`:

```toml
[nested]
date=1979-05-27
```

```console
$ knf lossy.toml --to json
```

`stdout`:

```text
{
  "nested": {
    "date": "1979-05-27"
  }
}
```

`stderr`:

```text
warning: conversion was lossy; rerun with --strict to find the affected keys
```

### Strict names the datetime key

`lossy.toml`:

```toml
[nested]
date=1979-05-27
```

```console
$ knf lossy.toml --to json --strict
? 1
```

`stderr`:

```text
error: nested.date: the JSON data model cannot represent TOML dates, times or datetimes
```

### JSON nulls are dropped in TOML

`lossy.json`:

```json
{"nested":{"drop":null}}
```

```console
$ knf lossy.json --to toml
```

`stdout`:

```text
[nested]
```

`stderr`:

```text
warning: conversion was lossy; rerun with --strict to find the affected keys
```

### Strict names the null key

`lossy.json`:

```json
{"nested":{"drop":null}}
```

```console
$ knf lossy.json --to toml --strict
? 1
```

`stderr`:

```text
error: nested.drop: TOML cannot represent null object fields
```

### Strict is silent when nothing is lost

`clean.json`:

```json
{"nested":{"a":1}}
```

```console
$ knf clean.json --to toml --strict
```

`stdout`:

```text
[nested]
a = 1
```

### Same-format JSON is lossless under strict

`same.json`:

```json
{"nothing":null,"big":18446744073709551615}
```

```console
$ knf same.json --to json --strict --compact
```

`stdout`:

```text
{"nothing":null,"big":18446744073709551615}
```

### Same-format TOML is lossless under strict

`same.toml`:

```toml
day = 1979-05-27
limit = inf
```

```console
$ knf same.toml --to toml --strict
```

`stdout`:

```text
day = 1979-05-27
limit = inf
```

### Unsigned integers above i64 become floats in lossy TOML

`big.json`:

```json
{"big":18446744073709551615}
```

```console
$ knf big.json --to toml
```

`stdout`:

```text
big = 18446744073709552000.0
```

`stderr`:

```text
warning: conversion was lossy; rerun with --strict to find the affected keys
```

### Strict rejects unsigned integers above i64 in TOML

`big.json`:

```json
{"big":18446744073709551615}
```

```console
$ knf big.json --to toml --strict
? 1
```

`stderr`:

```text
error: big: integer exceeds TOML's signed 64-bit range
```

### Null array entries fail conversion without strict

`null_array.json`:

```json
{"nested":[1,null,2]}
```

```console
$ knf null_array.json --to toml
? 1
```

`stderr`:

```text
error: nested[1]: TOML cannot represent null
```

### Strict requires an output format before reading inputs

```console
$ knf missing.json --strict
? 2
```

`stderr`:

```text
error: the following required arguments were not provided:
  --to <FORMAT>

Usage: knf --to <FORMAT> --strict <FILE>...

For more information, try '--help'.
```

### Strict is a no-op when the format does not change

`a.json`:

```json
{"n":null,"big":18446744073709551615}
```

```console
$ knf a.json --compact --to json --strict
```

`stdout`:

```text
{"n":null,"big":18446744073709551615}
```

### Same-format output without the flags is identical

`a.json`:

```json
{"n":null,"big":18446744073709551615}
```

```console
$ knf a.json --compact
```

`stdout`:

```text
{"n":null,"big":18446744073709551615}
```

## TOML values

### Datetimes survive a TOML round trip

`f.toml`:

```toml
name = "svc"
date = 1979-05-27T07:32:00Z
```

```console
$ knf f.toml
```

`stdout`:

```text
name = "svc"
date = 1979-05-27T07:32:00Z
```

### Datetimes survive an inline config

`f.toml`:

```toml
name = "svc"
date = 1979-05-27T07:32:00Z
```

```console
$ knf f.toml -c extra=1
```

`stdout`:

```text
name = "svc"
date = 1979-05-27T07:32:00Z
extra = 1
```

### A null inline config overwritten before TOML output

`f.toml`:

```toml
a = 0
```

```console
$ knf f.toml -c a=null -c a=1
```

`stdout`:

```text
a = 1
```

### Inline typing uses input-format values with a string fallback

```console
$ knf --to toml -c proxy=null -c day=1979-05-27 -c "db={host='local'}" -c large=18446744073709551615 -c 'bad=[a,b]' -c limit=inf
```

`stdout`:

```text
proxy = "null"
day = 1979-05-27
large = "18446744073709551615"
bad = "[a,b]"
limit = inf

[db]
host = "local"
```

### Inline literals accept surrounding whitespace

```console
$ knf --to toml -c 'port= 8080 ' -c 'enabled=true ' -c 'ratio=1.0 ' -c 'fallback= text ' -c "quoted= ' text ' "
```

`stdout`:

```text
port = 8080
enabled = true
ratio = 1.0
fallback = " text "
quoted = " text "
```

### Environment values share inline typing and remain terminal

`a.toml`:

```toml
day = '${env:KNF_TEST_DAY}'
null = '${env:KNF_TEST_NULL}'
raw = 'x:${env:KNF_TEST_DAY}'
terminal = '${env:KNF_TEST_TERMINAL}'
```

```console
$ KNF_TEST_DAY=1979-05-27 KNF_TEST_NULL=null KNF_TEST_TERMINAL='${missing}' knf a.toml --interpolate
```

`stdout`:

```text
day = 1979-05-27
null = "null"
raw = "x:1979-05-27"
terminal = "${missing}"
```

## An empty key is quoted in a conversion error

The top-level `x` is not the null one.

`input.json`:

```json
{"":{"x":null},"x":42}
```

```console
$ knf input.json --to toml --strict
? 1
```

`stderr`:

```text
error: [""].x: TOML cannot represent null object fields
```
