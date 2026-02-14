# AGENTS.md for `rust/ais-rs`

Scope: this file applies to the entire `rust/ais-rs` workspace tree.

## Documentation contract for crates

- Any code change under `crates/<crate-name>/` must be accompanied by a corresponding update to `crates/<crate-name>/README.md`.
- Any newly added crate under `crates/` must include a `README.md` in the crate root at creation time.
- Keep crate READMEs practical and integration-oriented:
  - crate purpose and boundaries
  - public API entry points
  - dependencies on other workspace crates
  - current implementation status / known gaps

## Change hygiene

- Prefer small, incremental README updates with each crate change.
- Do not postpone README updates to a later cleanup PR.

