# AGENTS.md (knf-py)

PyO3 bindings (module `knf._knf`). The `pyknf` wheel is built from the root `pyproject.toml`; Python sources and tests are in `/python` at the repo root.

## Code Rules

- Native Python-to-value conversion lives in `src/from_py.rs`.

## Invariants

- **Datetimes:** Return datetime/date/time objects with fixed offsets when present; truncate nanoseconds to microseconds, and report unrepresentable datetime values with key paths.

## Tests

- Run with `just test-py` (maturin develop + pytest).
- `python/tests/test_exec.py`: `--exec` / `knf.config()`.
- `python/tests/test_in_memory_merge.py`: `deep_merge`.
