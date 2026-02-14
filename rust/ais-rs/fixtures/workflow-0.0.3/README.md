# Workflow 0.0.3 conformance fixtures

This fixture set tracks `ais-flow/0.0.3` behavior checks shared by SDK/Runner/Engine.

## Current coverage

- `imports/valid.json`: imports declaration and node protocol closure.
- `imports/invalid-missing-path.json`: missing `imports.protocols[].path` semantic error.
- `imports/invalid-bad-protocol-format.json`: invalid `imports.protocols[].protocol` format.
- `imports/invalid-node-not-imported.json`: node protocol missing from `imports.protocols`.
- `assert/success.json`: compile preserves `assert` + `assert_message`.
- `assert/fail-invalid-cel.json`: compile rejects invalid assert CEL syntax.
- `assert/type-error.json`: compile rejects non-boolean `assert.lit`.
- `calculated_overrides/chain.json`: chained overrides keep deterministic dependency order.
- `calculated_overrides/missing-ref.json`: compile reports missing calculated dependency.
- `calculated_overrides/cycle.json`: compile reports dependency cycle.
- `preflight/simulate.json`: preflight simulate map is present for workflow-level simulate semantics.
- `policy/allowlist-evm.json`: policy allowlist payload fixture for EVM workflow path.
- `policy/need-user-confirm.json`: policy requiring interactive confirm payload fixture.
- `full-path/pass.json`: combined imports/assert/calculated_overrides/preflight/policy happy path.

## Notes

- This set is intentionally reusable across SDK/Runner/Engine tests.
- For engine-specific execution semantics (`condition`, `until`, `retry`, `timeout_ms`), add dedicated fixtures under this directory when corresponding runtime tasks are implemented.
