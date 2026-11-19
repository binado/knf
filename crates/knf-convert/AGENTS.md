# AGENTS.md (knf-convert)

Output conversion between native JSON and TOML values. No CLI or Python dependencies.

- Conversion happens after merge and interpolation; this crate never merges.
- TOML datetimes stay native; never route values through JSON.
- Unit tests cover conversion and lossy-warning cases.
