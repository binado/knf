# Context

## Context values are not merged and see final overrides

`foo.toml`:

```toml
host = 'base'
url = '${service.url}'
port = '${service.port}'
```

`bar.toml`:

```toml
host = 'prod'
```

`config.toml`:

```toml
host = 'shared'
unused = '${missing}'
[service]
url = 'https://${host}/api'
port = 5432
```

### Configs override the context

```console
$ knf foo.toml bar.toml -i --with config.toml
```

`stdout`:

```text
host = "prod"
url = "https://prod/api"
port = 5432
```

### Set values override the context

```console
$ knf foo.toml bar.toml -i --with config.toml -c host=inline
```

`stdout`:

```text
host = "inline"
url = "https://inline/api"
port = 5432
```

## Context is independent of filtering and merge options

`foo/base.json`:

```json
{"db":{"host":"prod","port":1}}
```

`foo/bar/target.json`:

```json
{"db":{"host":"prod"},"port":"${db.port}"}
```

`config.json`:

```json
{"db":{"host":false,"port":5432}}
```

```console
$ knf foo/base.json foo/bar/target.json -G '{base,target}.json' --shallow db -i --with config.json --compact
```

`stdout`:

```text
{"db":{"host":"prod"},"port":5432}
```

## Output format

`config.toml`:

```toml
port = 5432
```

`dated.toml`:

```toml
day = 1979-05-27
```

### Inline values select the context format

```console
$ knf -i --with config.toml -c 'copy=${port}'
```

`stdout`:

```text
copy = 5432
```

### Empty output uses the context format

```console
$ knf -i --with config.toml
```

`stdout`:

```text

```

### Context datetimes stay datetimes and are not emitted

```console
$ knf -i --with dated.toml -c 'copy=${day}'
```

`stdout`:

```text
copy = 1979-05-27
```

## List files skips context IO and format checks

### With input files

```console
$ knf a.json -i --with missing.toml --list-files
```

`stdout`:

```text
a.json
```

### Without input files

```console
$ knf -i --with missing.toml --list-files
```

## Argument errors are usage errors before IO

### Context without interpolation

```console
$ knf missing.json --with context.json
? 2
```

`stderr`:

```text
error: the following required arguments were not provided:
  --interpolate

Usage: knf --interpolate --with <FILE> <FILE>...

For more information, try '--help'.
```

### Missing context file

```console
$ knf -i --with
? 2
```

`stderr`:

```text
error: a value is required for '--with <FILE>' but none was supplied

For more information, try '--help'.
```

### Repeated context

```console
$ knf -i --with a.json --with b.json
? 2
```

`stderr`:

```text
error: the argument '--with <FILE>' cannot be used multiple times

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

## Formats are validated before document reads

```console
$ knf missing.json -i --with missing.toml
? 1
```

`stderr`:

```text
error: inputs mix JSON and TOML formats; configs must use one format
help: inputs and the --with file must share one format
```

## Context files are parsed and validated even when unused

`a.json`:

```json
{}
```

`bad.json`:

```json
{
```

`array.json`:

```json
[]
```

`unresolved.json`:

```json
{"unused":"${missing}"}
```

`config/base.toml`:

```toml
a = 1
```

### Invalid JSON

```console
$ knf a.json -i --with bad.json
? 1
```

`stderr`:

```text
error: loading `bad.json`
  caused by: invalid JSON
  caused by: EOF while parsing an object at line 2 column 0
```

### Not an object

```console
$ knf a.json -i --with array.json
? 1
```

`stderr`:

```text
error: loading `array.json`
  caused by: expected an object at the top level, found array
```

### Missing file

```console
$ knf a.json -i --with missing.json
? 1
```

`stderr`:

```text
error: loading `missing.json`
  caused by: No such file or directory (os error 2)
```

### Directory

```console
$ knf a.json -i --with config
? 1
```

`stderr`:

```text
error: `config` is a directory; knf takes files as configs
help: --with takes one file
```

### Unresolved references in unused values are accepted

```console
$ knf a.json -i --with unresolved.json --compact
```

`stdout`:

```text
{}
```

## Mixed input formats

`a.toml`:

```toml
a = 1
```

`b.json`:

```json
{}
```

```console
$ knf a.toml -i --with b.json
? 1
```

`stderr`:

```text
error: inputs mix JSON and TOML formats; configs must use one format
help: inputs and the --with file must share one format
```

## Interpolation diagnostics name context paths

`a.json`:

```json
{"copy":"${settings.bad}"}
```

`config.json`:

```json
{"settings":{"bad":"${missing}"}}
```

```console
$ knf a.json -i --with config.json
? 1
```

`stderr`:

```text
error: unresolved reference
  --> settings.bad (context): `missing`
help: `${key.path}` names a key in the merged document or --with context, `${env:NAME}` an environment variable
help: (context) paths are in `config.json`
help: drop --interpolate and --with to pass `${...}` through untouched
```

## Environment values reach the context

`a.json`:

```json
{"copy":"${value}"}
```

`config.json`:

```json
{"value":"${env:KNF_CONTEXT_VALUE}"}
```

```console
$ KNF_CONTEXT_VALUE=8080 knf a.json -i --with config.json --compact
```

`stdout`:

```text
{"copy":8080}
```

## A problem inside the context names the context file

`doc.json`:

```json
{"settings":{"other":1},"copy":"${settings.url}"}
```

`ctx.json`:

```json
{"settings":{"url":"${missing}"}}
```

```console
$ knf doc.json -i --with ctx.json
? 1
```

`stderr`:

```text
error: unresolved reference
  --> settings.url (context): `missing`
help: `${key.path}` names a key in the merged document or --with context, `${env:NAME}` an environment variable
help: (context) paths are in `ctx.json`
help: drop --interpolate and --with to pass `${...}` through untouched
```
