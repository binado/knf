# Shallow

## Shared inputs

`base.toml`:

```toml
plugins = ["auth"]
[db]
host = "local"
port = 5432
```

`prod.toml`:

```toml
plugins = ["metrics"]
[db]
host = "prod"
```

### Top-level tables are taken whole

```console
$ knf base.toml prod.toml --shallow=* --compact
```

`stdout`:

```text
plugins = ["metrics"]

[db]
host = "prod"
```

### A single config is the identity

```console
$ knf base.toml --shallow=*
```

`stdout`:

```text
plugins = ["auth"]

[db]
host = "local"
port = 5432
```

### Inline configs are shallow too

```console
$ knf base.toml --shallow=* -c db.host=x
```

`stdout`:

```text
plugins = ["auth"]

[db]
host = "x"
```

### A pattern may precede the files

```console
$ knf --shallow '*' base.toml prod.toml
```

`stdout`:

```text
plugins = ["metrics"]

[db]
host = "prod"
```

## Nested inputs

`base.toml`:

```toml
[db]
host = "local"
[db.pool]
min = 1
max = 5
[app.pool]
min = 1
max = 5
```

`prod.toml`:

```toml
[db.pool]
max = 9
[app.pool]
max = 9
```

### A path pattern keeps everything else deep

```console
$ knf base.toml prod.toml --shallow=db.* --compact
```

`stdout`:

```text
[db]
host = "local"

[db.pool]
max = 9

[app.pool]
min = 1
max = 9
```

### Braces select multiple objects

```console
$ knf base.toml prod.toml '--shallow={db,app}.*' --compact
```

`stdout`:

```text
[db]
host = "local"

[db.pool]
max = 9

[app.pool]
max = 9
```

### An exact object selector drops omitted children

```console
$ knf base.toml prod.toml --shallow db
```

`stdout`:

```text
[db.pool]
max = 9

[app.pool]
min = 1
max = 9
```

### Glob, inline configs and interpolation compose

```console
$ knf base.toml --shallow 'db.*' -c 'db.pool={max=9}' -c 'copy=${db.pool}' --interpolate
```

`stdout`:

```text
[db]
host = "local"

[db.pool]
max = 9

[app.pool]
min = 1
max = 5

[copy]
max = 9
```

## Quoted keys are distinct from nested paths in JSON

`base.json`:

```json
{"foo.bar":{"old":1},"foo/bar":{"old":1},"foo\\bar":{"old":1},"servers[0]":{"old":1},"*":{"old":1},"":{"old":1},"foo":{"bar":{"old":1}}}
```

`over.json`:

```json
{"foo.bar":{"new":2},"foo/bar":{"new":2},"foo\\bar":{"new":2},"servers[0]":{"new":2},"*":{"new":2},"":{"new":2},"foo":{"bar":{"new":2}}}
```

```console
$ knf base.json over.json --shallow "{'foo.bar','foo/bar','foo\\\\bar','servers[0]','*',''}" --compact
```

`stdout`:

```text
{"foo.bar":{"new":2},"foo/bar":{"new":2},"foo\\bar":{"new":2},"servers[0]":{"new":2},"*":{"new":2},"":{"new":2},"foo":{"bar":{"old":1,"new":2}}}
```

## Quoted keys are distinct from nested paths in TOML

`base.toml`:

```toml
"foo.bar" = { old = 1 }
"foo/bar" = { old = 1 }
"foo\\bar" = { old = 1 }
"servers[0]" = { old = 1 }
"*" = { old = 1 }
"" = { old = 1 }
foo = { bar = { old = 1 } }
```

`over.toml`:

```toml
"foo.bar" = { new = 2 }
"foo/bar" = { new = 2 }
"foo\\bar" = { new = 2 }
"servers[0]" = { new = 2 }
"*" = { new = 2 }
"" = { new = 2 }
foo = { bar = { new = 2 } }
```

```console
$ knf base.toml over.toml --shallow "{'foo.bar','foo/bar','foo\\\\bar','servers[0]','*',''}"
```

`stdout`:

```text
["foo.bar"]
new = 2

["foo/bar"]
new = 2

['foo\bar']
new = 2

["servers[0]"]
new = 2

["*"]
new = 2

[""]
new = 2

[foo.bar]
old = 1
new = 2
```

## Pattern errors precede file IO

`--shallow` needs exactly one non-empty, valid pattern; `missing.toml` is never opened.

### Missing pattern

```console
$ knf missing.toml --shallow
? 2
```

`stderr`:

```text
error: a value is required for '--shallow <PATTERN>' but none was supplied

For more information, try '--help'.
```

### Empty pattern

```console
$ knf missing.toml --shallow=
? 2
```

`stderr`:

```text
error: invalid value '' for '--shallow <PATTERN>': empty key-path glob; use '*' to select top-level keys

For more information, try '--help'.
```

### Repeated pattern

```console
$ knf missing.toml --shallow=a --shallow=b
? 2
```

`stderr`:

```text
error: the argument '--shallow <PATTERN>' cannot be used multiple times

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### Unclosed quote

```console
$ knf missing.toml "--shallow='unclosed"
? 2
```

`stderr`:

```text
error: invalid value ''unclosed' for '--shallow <PATTERN>': unclosed quote at byte 0

For more information, try '--help'.
```

### Unclosed brace

```console
$ knf missing.toml '--shallow={a,b'
? 2
```

`stderr`:

```text
error: invalid value '{a,b' for '--shallow <PATTERN>': unclosed brace expansion at byte 0; missing '}' (to match a literal '{', escape it as '\{' or '[{]')

For more information, try '--help'.
```

### Unclosed character class

```console
$ knf missing.toml '--shallow=servers[0'
? 2
```

`stderr`:

```text
error: invalid value 'servers[0' for '--shallow <PATTERN>': unclosed character class at byte 7; missing ']' (to match a literal '[', escape it as '\[' or '[[]')

For more information, try '--help'.
```
