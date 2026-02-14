# AGENTS.md (repository root)

Scope: this file applies to the whole repository unless a deeper `AGENTS.md` overrides it.

## Rust workspace documentation rule (`rust/ais-rs`)

- For any code change under `rust/ais-rs/crates/<crate-name>/`, update `rust/ais-rs/crates/<crate-name>/README.md` in the same change.
- For any new crate under `rust/ais-rs/crates/`, create `README.md` in that crate at creation time.
- Keep crate README content integration-focused:
  - crate purpose and boundaries
  - public API entry points
  - dependencies on other workspace crates
  - current implementation status and known gaps

