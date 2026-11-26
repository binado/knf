# Glob filters

## Modes and ordering

`config/a.prod.toml`:

```toml
value = 1
```

`config/b.prod.toml`:

```toml
value = 2
```

`config/c.dev.toml`:

```toml
invalid ignored TOML
```

### -G matches the filename and preserves order and duplicates

```console
$ knf config/b.prod.toml config/c.dev.toml config/a.prod.toml config/b.prod.toml -G '*.prod.toml' --list-files
```

`stdout`:

```text
config/b.prod.toml
config/a.prod.toml
config/b.prod.toml
```

### --glob-filename is the long form of -G

```console
$ knf config/b.prod.toml config/c.dev.toml config/a.prod.toml config/b.prod.toml --glob-filename '*.prod.toml' --list-files
```

`stdout`:

```text
config/b.prod.toml
config/a.prod.toml
config/b.prod.toml
```

### -G drops excluded inputs before loading

```console
$ knf config/a.prod.toml config/c.dev.toml config/b.prod.toml -G '*.prod.toml'
```

`stdout`:

```text
value = 2
```

### -g matches the whole path, not the filename

```console
$ knf config/a.prod.toml -g '*.prod.toml' --list-files
```

### --glob is the long form of -g

```console
$ knf config/a.prod.toml --glob 'config/**/*.prod.toml' --list-files
```

`stdout`:

```text
config/a.prod.toml
```

### -g with ** matches nested directories

```console
$ knf config/a.prod.toml -g 'config/**/*.prod.toml' --list-files
```

`stdout`:

```text
config/a.prod.toml
```

### -g does not match a ./ prefixed input against a bare pattern

```console
$ knf ./config/a.prod.toml -g 'config/*.prod.toml' --list-files
```

### -g matches a ./ prefixed input against a ./ prefixed pattern

```console
$ knf ./config/a.prod.toml -g './config/*.prod.toml' --list-files
```

`stdout`:

```text
./config/a.prod.toml
```

## Pattern syntax

### Matching is case-sensitive

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G 'prod.toml' --list-files
```

`stdout`:

```text
foo/prod.toml
```

### Braces select alternatives

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G '{defaults,prod}.toml' --list-files
```

`stdout`:

```text
foo/defaults.toml
foo/prod.toml
```

### Character classes and ? match single characters

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G '[pd]??.toml' --list-files
```

`stdout`:

```text
foo/dev.toml
```

### A negated pattern excluding everything selects nothing

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G '!*.toml' --list-files
```

### Negation inverts a brace pattern

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G '!{defaults,prod}.toml' --list-files
```

`stdout`:

```text
foo/Prod.toml
foo/dev.toml
```

### A leading dot is literal

```console
$ knf foo/defaults.toml foo/prod.toml foo/Prod.toml foo/dev.toml -G '.prod.toml' --list-files
```

### A negated pattern cannot match inputs without a filename

```console
$ knf . .. -G '!prod.toml' --list-files
```

### ? matches one character, not one byte

```console
$ knf é.toml -G '?.toml' --list-files
```

### ?? matches two characters

```console
$ knf é.toml -G '??.toml' --list-files
```

`stdout`:

```text
é.toml
```

## Excluded inputs

`prod.toml`:

```toml
value = 1
```

`bad.json`:

```json
invalid JSON
```

### Excluded inputs are not loaded or inspected

```console
$ knf bad.json missing.json prod.toml -G '*.toml'
```

`stdout`:

```text
value = 1
```

### Excluded inputs do not infer the output format

```console
$ knf bad.json missing.json prod.toml -G '*.toml' -c value=2
```

`stdout`:

```text
value = 2
```

## Empty selections

### -g selecting nothing yields an empty document

```console
$ knf missing.toml -g '*.json' --to json --compact
```

`stdout`:

```text
{}
```

### -g selecting nothing still applies -c

```console
$ knf missing.toml -g '*.json' -c value=2 --to json --compact
```

`stdout`:

```text
{"value":2}
```

### -g selecting nothing lists no files

```console
$ knf missing.toml -g '*.json' --list-files
```

### -g with no inputs yields an empty document

```console
$ knf -g '*.json' --to json --compact
```

`stdout`:

```text
{}
```

### -G selecting nothing yields an empty document

```console
$ knf missing.toml -G '*.json' --to json --compact
```

`stdout`:

```text
{}
```

### -G selecting nothing still applies -c

```console
$ knf missing.toml -G '*.json' -c value=2 --to json --compact
```

`stdout`:

```text
{"value":2}
```

### -G selecting nothing lists no files

```console
$ knf missing.toml -G '*.json' --list-files
```

### -G with no inputs yields an empty document

```console
$ knf -G '*.json' --to json --compact
```

`stdout`:

```text
{}
```

## Usage errors precede filesystem access

### -g without a pattern

```console
$ knf -g
? 2
```

`stderr`:

```text
error: a value is required for '--glob <PATTERN>' but none was supplied

For more information, try '--help'.
```

### -G without a pattern

```console
$ knf -G
? 2
```

`stderr`:

```text
error: a value is required for '--glob-filename <PATTERN>' but none was supplied

For more information, try '--help'.
```

### -g and -G cannot be combined

```console
$ knf -g '*' -G '*'
? 2
```

`stderr`:

```text
error: the argument '--glob <PATTERN>' cannot be used with '--glob-filename <PATTERN>'

Usage: knf --glob <PATTERN> [FILE]...

For more information, try '--help'.
```

### -g cannot be repeated

```console
$ knf -g '*' -g '*'
? 2
```

`stderr`:

```text
error: the argument '--glob <PATTERN>' cannot be used multiple times

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### -G cannot be repeated

```console
$ knf -G '*' -G '*'
? 2
```

`stderr`:

```text
error: the argument '--glob-filename <PATTERN>' cannot be used multiple times

Usage: knf [OPTIONS] [FILE]...

For more information, try '--help'.
```

### An invalid -g pattern is rejected before reading

```console
$ knf missing.toml -g '[broken'
? 2
```

`stderr`:

```text
error: invalid value '[broken' for '--glob <PATTERN>': unclosed character class at byte 0; missing ']' (to match a literal '[', escape it as '\[' or '[[]')

For more information, try '--help'.
```

### An invalid -G brace pattern is rejected before reading

```console
$ knf missing.toml -G '{broken'
? 2
```

`stderr`:

```text
error: invalid value '{broken' for '--glob-filename <PATTERN>': unclosed brace expansion at byte 0; missing '}' (to match a literal '{', escape it as '\{' or '[{]')

For more information, try '--help'.
```

### A trailing backslash is rejected before reading

```console
$ knf missing.toml -g 'trailing\'
? 2
```

`stderr`:

```text
error: invalid value 'trailing\' for '--glob <PATTERN>': trailing backslash at byte 8 has no character to escape (to match a literal '\', use '\\')

For more information, try '--help'.
```

### -c errors surface before reading excluded inputs

```console
$ knf missing.toml -G none -c 'a[0]=1'
? 2
```

`stderr`:

```text
error: invalid value 'a[0]=1' for '-c <KEY.PATH=VALUE>': `a[0]` contains an array index; merge paths take keys only
help: -c takes KEY.PATH=VALUE; an index like servers[0] can be read
      by a ${...} reference but never written — put the value in a file instead

For more information, try '--help'.
```
