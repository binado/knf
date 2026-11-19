# knf-mdtest

Test-only harness that runs command-line tests written as Markdown. Each `.md` file is one libtest case; its sections run a binary against inline input files and compare stdout, stderr and the exit code. Not published.

## Wiring

```rust
use std::path::Path;

fn knf(path: &Path) -> datatest_stable::Result<()> {
    mdtest::run_file(
        path,
        &mdtest::Config { program: "knf", bin: Path::new(env!("CARGO_BIN_EXE_knf")) },
    )
}

datatest_stable::harness! { { test = knf, root = "resources/mdtest", pattern = r"\.md$" } }
```

The test target needs `harness = false` and a dev-dependency on `datatest-stable`.

## Format

- Each heading is a section. A section with a `console` block is a test; child sections inherit the input files of their parents and may override them by path.
- `` `path/to/file`: `` followed by a fence writes an input file (relative, no `..`) into a fresh temp directory.
- The `console` block holds one `$ [NAME=VALUE ...] <program> <args>` line (shell-style quoting) and an optional `? N` line for the expected exit code (default 0).
- `` `stdout`: `` and `` `stderr`: `` followed by a fence give the exact expected output. An absent block means empty output.
- Strict parsing: fences must not be indented, must be terminated and must be labelled; a test section holds exactly one `console` block and no subsections; output blocks require a `console` block.
- The command runs with a cleared environment (only `PATH` and the declared variables) and the temp directory as working directory.

## Environment

- `MDTEST_FILTER=<substring>` runs only sections whose name contains it.
- `MDTEST_UPDATE=1` rewrites `stdout`/`stderr` blocks to match the actual output; review the diff.

## Credits

The format and workflow are modelled on the `mdtest` framework of [ty](https://github.com/astral-sh/ty), the Python type checker in Astral's [ruff](https://github.com/astral-sh/ruff) repository ([`crates/ty_test`](https://github.com/astral-sh/ruff/tree/main/crates/ty_test)). No code is copied; any mistakes are ours.
