# `ais-core`

Core shared primitives for AIS Rust crates.

## Responsibility

- Field path representation and parsing
- Structured issue shape and stable sorting
- Stable JSON canonicalization and stable hash helpers
- Runtime patch model, guard policy, and patch apply audit

## Public entry points

- `FieldPath`, `FieldPathSegment`
- `StructuredIssue`, `IssueSeverity`
- `stable_json_bytes`, `stable_hash_hex`
- `RuntimePatch`, `RuntimePatchGuardPolicy`
- `apply_runtime_patches`, `check_runtime_patch_path_allowed`

## Dependencies

- No dependency on higher-level crates (`ais-sdk`, `ais-engine`, `ais-runner`)
- Intended to be reused by schema/sdk/engine layers

## Test layout

- Unit tests live in dedicated `*_test.rs` files in `src/` (and `src/runtime_patch/`).

## Current status

- Implemented:
  - field path
  - issues
  - stable json/hash
  - runtime patch + guard + apply audit
- Planned next:
  - `ais-json/1` codec (`AISRS-CORE-005`)
  - redaction modes (`AISRS-CORE-006`)
