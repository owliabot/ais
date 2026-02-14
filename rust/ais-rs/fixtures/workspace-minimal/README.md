# AISRS-FIX-001 workspace fixtures

This folder provides minimal workspace fixtures (`protocol/pack/workflow`) for Rust SDK/runner tests.

## Cases

- `valid-evm-policy`
  - Covers `includes`, `chain_scope`, `token_policy.allowlist`, and policy-gate driven `need_user_confirm` paths.
  - Expected: schema parse pass + `validate_workspace_references` pass.
- `valid-solana-read`
  - Covers Solana workspace wiring for `query_ref` and `solana:*` chain scope.
  - Expected: schema parse pass + `validate_workspace_references` pass.
- `invalid-chain-scope`
  - Pack includes protocol but limits `chain_scope` to another chain.
  - Expected: `workspace.workflow.chain_scope_violation`.
- `patch-guard`
  - Workspace docs + command JSONL fixtures to test patch-guard behavior in engine/runner loops.
  - `commands-invalid-patch.jsonl`: writes under `nodes.*` (should be rejected by default guard).
  - `commands-valid-patch.jsonl`: writes under `inputs.*` (allowed by default guard).
