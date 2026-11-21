# AGENTS.md

Repo-wide guidance for AI agents. Crate-specific rules live in `crates/<crate>/AGENTS.md`.

- Commits must follow Conventional Commits.
- Keep `README.md` examples and tables in sync with CLI and Python API changes. Keep the README and comments concise: detail belongs in `--help`, rustdoc and tests.
- Don't use code comments to enforce invariance or patterns. Those should be manifested in the code design.

## Commands

MSRV is Rust 1.88; the opt-in `yaml` feature (`knf-cli`, `knf-py`) needs 1.89.

```bash
just ci                                 # format-check + lint + test + build + doc (all features)
just test-py                            # maturin develop + pytest in active venv
just bless                              # rewrite expected output in Markdown fixtures (MDTEST_UPDATE=1)
cargo test -p knf-core --lib            # fast inner loop: unit tests only
cargo test -p knf-cli --test cli <name> # run a single CLI integration test
```

## Crates

- `crates/knf-core/`: The library (crate name `knf`). Ordered values, merge, interpolation. No `clap`.
- `crates/knf-convert/`: File loading, parsing, output conversion and emission. No CLI or Python dependencies.
- `crates/knf/`: The CLI binary (`knf-cli`). No `pyo3`.
- `crates/mdtest/`: Markdown fixture harness (`knf-mdtest`, dev-only, not published).
- `crates/knf-py/`: PyO3 bindings (module `knf._knf`). Python sources and tests are in `python/`; the `pyknf` wheel is built from the root `pyproject.toml`.
