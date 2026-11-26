# Dry run

## Files precede inline assignments regardless of argument order

```console
$ knf -c "server. 'port' = 8080" ./base.toml -c 'server.port=9090' absent.json ./base.toml --dry-run
```

`stdout`:

```text
[dry run] Skipped merging configs in this order (lowest to highest precedence):
1. ./base.toml
2. absent.json
3. ./base.toml
4. server. 'port' = 8080
5. server.port=9090
```

## Inline assignments alone need no output format

```console
$ knf --dry-run -c 'url="https://example.org/?q=v"'
```

`stdout`:

```text
[dry run] Skipped merging configs in this order (lowest to highest precedence):
1. url="https://example.org/?q=v"
```
