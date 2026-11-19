# AGENTS.md (knf CLI)

The CLI binary (`knf-cli`): argv parsing, stderr formatting and `--exec` process launch. No `pyo3`. Merge semantics live in `knf-core`; keep this crate thin.

## Adding a CLI test

Prefer a Markdown fixture in `resources/mdtest/*.md` (run by `tests/mdtest.rs`; format in [`crates/mdtest/README.md`](../mdtest/README.md)). Each heading is a section; a section with a `console` block is a test, and child sections inherit parent input files.

````markdown
## Unresolved reference

`broken.toml`:

```toml
a = "${missing}"
```

```console
$ HOME=/x knf broken.toml -i
? 1
```

`stderr`:

```text
error: unresolved reference `${missing}`
```
````

- Write the console block, then run `just bless` to fill in `stdout`/`stderr` and review the diff.
- Tests about process behaviour (`--exec`, signals), non-UTF-8 paths or platform `cfg` stay in `tests/cli.rs`.
