# AGENTS.md (knf CLI)

The CLI binary (`knf-cli`): argv parsing, stderr formatting and `--exec` process launch. No `pyo3`. Merge semantics live in `knf-core`; keep this crate thin.

## Code Rules

- Flag-specific `help:` lines belong in `src/explain.rs`; library errors never name flags (see `knf-core/AGENTS.md`).
- Do not read the process environment directly; go through `ProcessEnv`.

## Invariants

- **Formats:** Inferred mixed input formats fail before document I/O; `-f` overrides parsing for every input.
- **Output:** Defaults to the input format. Cross-format conversion (via `knf-convert`) happens after merge and interpolation.

## Tests

- `tests/cli.rs`: CLI integration tests. Use `with_env`, never ambient env. Stderr snapshots live in `tests/snapshots/` (`just review`).
- `tests/public_api.rs`: Guards against missing re-exports.
