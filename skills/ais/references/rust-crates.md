# AIS Rust Crates Summary

Workspace: `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/`
Source READMEs: `/home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs/crates/*/README.md`

## Build and test

```bash
cd /home/ocbot/.openclaw/workspace/repos/ais/rust/ais-rs
cargo build --workspace
cargo test --workspace
```

## Core crates

- `ais-core`: shared primitives (field paths, structured issues, stable JSON/hash, runtime patch guard/apply).
- `ais-schema`: embedded schema registry and schema validation adapter.
- `ais-cel`: CEL tokenizer/parser/evaluator and numeric model (`BigInt`/decimal support).
- `ais-sdk`: parsing and typed documents (protocol/pack/workflow/plan/catalog/plan-sketch), resolver context, semantic/workspace validation, and compile/dry-run helpers.
- `ais-engine`: execution runtime primitives (event/command/checkpoint contracts, scheduler, policy gate, router/executor boundaries, replay/diff support).
- `ais-llm`: provider-agnostic typed LLM tool-calling layer and provider factory/chain orchestration.
- `ais-runner`: CLI wrapper for running plans/workflows/replay/agent loops and wiring config + executors.

## Execution crates

- `ais-evm-executor`: EVM execution handlers (`evm_read`, `evm_call`, `evm_rpc`) for `eip155:*`, backed by Alloy.
- `ais-solana-executor`: Solana execution handlers (`solana_read`, `solana_instruction`) and signer/RPC abstractions.
- `ais-offchain-executor`: off-chain HTTP plugin execution (`offchain_apy_query`) with domain allowlist and retry controls.

## Crate roles at a glance

- Authoring/parsing path: `ais-schema` + `ais-core` + `ais-cel` + `ais-sdk`
- Planning/execution path: `ais-sdk` -> `ais-engine` -> executor crates
- Agent/runtime path: `ais-llm` + `ais-runner` + `ais-engine`

## Notable public entry points by crate

### `ais-sdk`
- Parse: `parse_document`, `parse_document_with_options`
- Resolve: `ResolverContext`, `evaluate_value_ref`, `resolve_action_ref`, `resolve_query_ref`
- Validate: `validate_document_semantics`, `validate_workspace_references`, `validate_workflow_document`
- Catalog: `build_catalog`, `build_catalog_index`, `get_executable_candidates`
- Planner: `compile_plan_skeleton`, `compile_plan_sketch`, `compile_workflow`, `dry_run_json`, `dry_run_text`

### `ais-engine`
- Events/commands/checkpoints: JSONL encode/decode and persistence helpers
- Router/execution: `Executor`, `RouterExecutor`, execution type capability helpers
- Policy and safety: policy-gate extraction/enforcement, confirmation summary/hash
- Runtime loop: `run_plan_once`, scheduling, replay and plan diff helpers

### `ais-llm`
- Provider trait and request/response model for tool-calling
- Provider registry/factory for OpenAI-compatible and Anthropic adapters
- Multi-provider chain with retry/fallback policies

### `ais-core`
- `FieldPath` parsing and handling
- `StructuredIssue` and severity model
- Stable canonicalization/hashing helpers
- Runtime patch apply and write-path allowlist checks

### `ais-schema`
- Schema id constants (`versions::*`)
- `get_json_schema`
- `validate_schema_instance`

### `ais-cel`
- `tokenize`, `parse_expression`
- CEL AST and evaluation APIs (`evaluate_expression`, `evaluate_ast`, `CELEvaluator`)

## Where to inspect crate details

- Per crate: `crates/<crate-name>/README.md`
- Engine behavior/contracts: `crates/ais-engine/README.md`
- SDK types and compile logic: `crates/ais-sdk/README.md`
- Runner CLI and intent loop behavior: `crates/ais-runner/README.md`
