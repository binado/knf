# AGENTS.md (knf-convert)

Loads homogeneous JSON or TOML configs into `knf::Value` and emits resolved values. No CLI or Python dependencies.

- Conversion policy lives in `OutputFormat::emit`; this crate never merges.
- Keep scalar types (datetimes, unsigned, non-finite floats) until output selection; never route values through JSON.
- Tests: `tests/round_trip.rs` (emit and reparse), `tests/library.rs` (file loading), `tests/input.rs` (inline literals and formats).
