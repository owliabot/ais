# `ais-schema`

Schema constants, embedded JSON Schemas, and schema validation adapter.

## Responsibility

- Central schema version constants
- Embedded schema registry from repository `schemas/0.0.2`
- Validate JSON instances against known schema ids and map errors to `StructuredIssue`

## Public entry points

- `versions::*` schema constants
- `get_json_schema(schema_id)`
- `validate_schema_instance(schema_id, &serde_json::Value)`

## Dependencies

- Uses `ais-core` for issue and field-path mapping
- Does not depend on `ais-sdk`

## Test layout

- Unit tests live in dedicated `*_test.rs` files in `src/`.

## Current status

- Implemented:
  - constants
  - schema embedding
  - validation adapter
- Planned next:
  - optional richer schema-error metadata mapping if needed by runner UX
