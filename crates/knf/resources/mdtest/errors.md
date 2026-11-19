# Errors

## Unknown flags are usage errors

```console
$ knf --no-such-flag
? 2
```

`stderr`:

```text
error: unexpected argument '--no-such-flag' found

  tip: to pass '--no-such-flag' as a value, use '-- --no-such-flag'

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

## Removed options are usage errors

### --cascade

```console
$ knf --cascade foo/bar/config.toml
? 2
```

`stderr`:

```text
error: unexpected argument '--cascade' found

  tip: to pass '--cascade' as a value, use '-- --cascade'

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### -r

```console
$ knf -r foo/bar/config.toml
? 2
```

`stderr`:

```text
error: unexpected argument '-r' found

  tip: to pass '-r' as a value, use '-- -r'

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### --input-format

```console
$ knf --input-format json
? 2
```

`stderr`:

```text
error: unexpected argument '--input-format' found

  tip: a similar argument exists: '--interpolate'

Usage: knf --interpolate [FILE]...

For more information, try '--help'.
```

### --null-as

```console
$ knf --null-as none
? 2
```

`stderr`:

```text
error: unexpected argument '--null-as' found

  tip: to pass '--null-as' as a value, use '-- --null-as'

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### --accumulate

```console
$ knf --accumulate a.toml
? 2
```

`stderr`:

```text
error: unexpected argument '--accumulate' found

  tip: to pass '--accumulate' as a value, use '-- --accumulate'

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

## A malformed -c value is a usage error

```console
$ knf -c noequals
? 2
```

`stderr`:

```text
error: invalid value 'noequals' for '-c <KEY.PATH=VALUE>': expected KEY.PATH=VALUE

For more information, try '--help'.
```

## A missing file exits one

```console
$ knf nope.json
? 1
```

`stderr`:

```text
error: reading `nope.json`
  caused by: No such file or directory (os error 2)
```

## A non-object root is rejected by name

`a.json`:

```json
[1,2]
```

```console
$ knf a.json
? 1
```

`stderr`:

```text
error: a.json: expected an object at the top level, found array
```

## Directories

`config/base.toml`:

```toml
a = 1
```

`context.toml`:

```toml
b = 1
```

### A directory input hints at the glob

```console
$ knf config
? 1
```

`stderr`:

```text
error: `config` is a directory; knf takes files as layers
help: `knf config/*.toml` merges its files as layers
```

### The layer hint is kept with --with

```console
$ knf config -i --with context.toml
? 1
```

`stderr`:

```text
error: `config` is a directory; knf takes files as layers
help: `knf config/*.toml` merges its files as layers
```

## Mixed input formats

`a.json`:

```json
{"a":"${nope}"}
```

`b.toml`:

```toml
b = 1
```

### Layers must use one format

```console
$ knf a.json b.toml
? 1
```

`stderr`:

```text
error: inputs mix JSON and TOML formats; layers must use one format
help: merge JSON layers and TOML layers separately
```

### The format error precedes interpolation errors

```console
$ knf a.json b.toml --interpolate
? 1
```

`stderr`:

```text
error: inputs mix JSON and TOML formats; layers must use one format
help: merge JSON layers and TOML layers separately
```

## A -c array index is rejected before file IO

```console
$ knf missing.toml -c 'servers[0]=1'
? 1
```

`stderr`:

```text
error: `servers[0]` contains an array index; merge paths take keys only
help: -c takes KEY.PATH=VALUE; an index like servers[0] can be read
      by a ${...} reference but never written — put the value in a file instead
```

## Paths without a known extension

`app.conf`:

```json
{}
```

### An unknown extension gets a rename hint

```console
$ knf app.conf
? 1
```

`stderr`:

```text
error: cannot infer a format from `app.conf`
help: name the file with a .json or .toml extension
```

### A dash gets a rename hint

```console
$ knf -
? 1
```

`stderr`:

```text
error: cannot infer a format from `-`
help: name the file with a .json or .toml extension
```

## Runs without input files

### Need --to for a format

```console
$ knf
? 1
```

`stderr`:

```text
error: no input files to infer a format from
help: pass --to json or --to toml
```

### -c alone needs --to too

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

### -c fills a TOML document

```console
$ knf --to toml -c a=1
```

`stdout`:

```text
a = 1
```

### Missing and unmatched inputs are ignored when -c is given

```console
$ knf missing.json -g '*.toml' --to toml -c a=1
```

`stdout`:

```text
a = 1
```
