# Merge

## A single layer is a no-op

`a.json`:

```json
{"a":{"b":1},"n":null,"xs":[1,2]}
```

```console
$ knf a.json --compact
```

`stdout`:

```text
{"a":{"b":1},"n":null,"xs":[1,2]}
```

## Key order is preserved

### JSON

`a.json`:

```json
{"zebra":1,"apple":2,"middle":3}
```

```console
$ knf a.json --compact
```

`stdout`:

```text
{"zebra":1,"apple":2,"middle":3}
```

### TOML

`a.toml`:

```toml
zebra = 1
apple = 2
middle = 3
```

```console
$ knf a.toml
```

`stdout`:

```text
zebra = 1
apple = 2
middle = 3
```

## Integers above i64::MAX are exact

`a.json`:

```json
{"id":10000000000000000001,"max":18446744073709551615}
```

```console
$ knf a.json --compact
```

`stdout`:

```text
{"id":10000000000000000001,"max":18446744073709551615}
```

## Homogeneous layers

`base.toml`:

```toml
[server]
port = 80
host = 'local'
```

`over.toml`:

```toml
[server]
port = 443
```

```console
$ knf base.toml over.toml
```

`stdout`:

```text
[server]
port = 443
host = "local"
```

## Arrays replace and tables merge

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

```console
$ knf base.toml prod.toml
```

`stdout`:

```text
plugins = ["metrics"]

[db]
host = "prod"
port = 5432
```

## Inline configs apply last

`a.json`:

```json
{"server":{"port":80}}
```

### Over a file

```console
$ knf a.json -c server.port=8080 --compact
```

`stdout`:

```text
{"server":{"port":8080}}
```

### Selecting the format with `--to`

```console
$ knf -c a.b=1 --to json --compact
```

`stdout`:

```text
{"a":{"b":1}}
```

## A reference layer inherits and a later layer overrides

### JSON

`base.json`:

```json
{"defaults":{"port":1,"host":"h"},"staging":"${defaults}","prod":"${staging}"}
```

`over.json`:

```json
{"staging":{"port":2},"read":"${staging.port}"}
```

#### Deep merge

```console
$ knf base.json over.json -i --compact
```

`stdout`:

```text
{"defaults":{"port":1,"host":"h"},"staging":{"port":2,"host":"h"},"prod":{"port":2,"host":"h"},"read":2}
```

#### Shallow

```console
$ knf base.json over.json -i --shallow staging --compact
```

`stdout`:

```text
{"defaults":{"port":1,"host":"h"},"staging":{"port":2},"prod":{"port":2},"read":2}
```

### TOML

`base.toml`:

```toml
staging = '${defaults}'
prod = '${staging}'

[defaults]
port = 1
host = 'h'
```

`over.toml`:

```toml
read = '${staging.port}'

[staging]
port = 2
```

#### Deep merge

```console
$ knf base.toml over.toml -i
```

`stdout`:

```text
read = 2

[staging]
port = 2
host = "h"

[prod]
port = 2
host = "h"

[defaults]
port = 1
host = "h"
```

#### Shallow

```console
$ knf base.toml over.toml -i --shallow staging
```

`stdout`:

```text
read = 2

[staging]
port = 2

[prod]
port = 2

[defaults]
port = 1
host = "h"
```
