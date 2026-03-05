# AIS Rust Crates

Key crates in the AIS Rust workspace.

## `ais-runner`
- Responsibility: CLI/runtime orchestration for `run plan`, `run workflow`, `plan diff`, `replay`, `agent`; wires config/router/checkpoint/events/trace.
- Public entry points: binary `ais-runner`; config helpers (`load_runner_config`, `validate_runner_config`, router builders); command surfaces listed in crate README.
- Current status: implemented and actively expanded (agent loop, checkpoint idempotency, replay/diff/workflow/run paths, safety/approvals integration).

## `ais-sdk`
- Responsibility: parse AIS docs (JSON/YAML), typed document models, semantic/workspace validation, resolver/value-ref eval, catalog/candidate build, workflow/plan-sketch compile, dry-run.
- Public entry points: `parse_document*`, `AisDocument`, typed docs, resolver APIs (`ResolverContext`, `evaluate_value_ref*`, `resolve_*_ref`), validators, catalog builders, planners (`compile_workflow`, `compile_plan_sketch`, `dry_run_*`).
- Current status: implemented for protocol/pack/workflow/plan/catalog/plan-sketch, validation, deterministic compile flows, and dry-run tooling.

## `ais-engine`
- Responsibility: execution runtime contracts and loop: events/commands/checkpoints/JSONL, scheduler, policy gate, router/executor boundary, plan diff, replay, side-effect lifecycle.
- Public entry points: event/command/checkpoint codecs, `Executor` + `RouterExecutor`, policy APIs, `run_plan_once`, diff APIs, replay APIs.
- Current status: implemented with side-effect ledger/reconcile support, policy/confirmation pipeline, command dedupe, deterministic scheduler, replay/diff, and extensive hardening.

## `ais-core`
- Responsibility: shared primitives (field paths, structured issues, stable JSON/hash, runtime patch model/guard/apply).
- Public entry points: `FieldPath*`, `StructuredIssue`, `stable_json_bytes`, `stable_hash_hex`, runtime patch APIs.
- Current status: core primitives implemented; planned next in README includes codec/redaction follow-ups.

## `ais-schema`
- Responsibility: schema version constants, embedded schema registry, schema instance validation mapped to structured issues.
- Public entry points: `versions::*`, `get_json_schema`, `validate_schema_instance`.
- Current status: implemented, including embedded schemas for intent/planning tools/plan-sketch/side-effect contracts.

## `ais-cel`
- Responsibility: CEL lexer/parser/AST/evaluator with exact numeric behavior.
- Public entry points: `tokenize`, `parse_expression`, AST types, numeric types, `evaluate_expression`, `evaluate_ast`, `CELEvaluator`.
- Current status: parser/evaluator/builtins implemented; README notes next integration focus on broader SDK evaluation paths.

## `ais-llm`
- Responsibility: provider-agnostic typed LLM tool-calling boundary plus provider factory/chain orchestration.
- Public entry points: `LlmProvider`, request/response/tool-call models, scripted provider, provider config/registry/factory APIs.
- Current status: implemented adapters (OpenAI-compatible + Anthropic), retry/fallback chains, and robust HTTP error handling; streaming/async API still listed as a gap.

## `ais-evm-executor`
- Responsibility: EVM executor support for `evm_read`, `evm_call`, `evm_rpc` on `eip155:*`.
- Public entry points: provider/types/executor/signer/redaction modules; `EvmExecutor` (`ais_engine::Executor`).
- Current status: implemented read/call/rpc paths, side-effect emission/reconcile, Alloy-backed transport, timeout/redaction behavior.

## `ais-solana-executor`
- Responsibility: Solana executor support for `solana_read`, `solana_instruction`.
- Public entry points: RPC/provider abstractions, `SolanaExecutor`, signer and redaction modules.
- Current status: implemented execution paths, signer/send/confirm, side-effect emission/reconcile, and redaction coverage.

## `ais-offchain-executor`
- Responsibility: offchain HTTP plugin executor for `offchain_apy_query`.
- Public entry points: `OffchainApyExecutor`, config/request/client abstractions.
- Current status: implemented domain allowlist, retries, normalized outputs, side-effect mapping/reconcile hook; README lists advanced resiliency/auth modeling as future gaps.

## Crate relationships
- Authoring/validation path: `ais-core` + `ais-schema` + `ais-cel` + `ais-sdk`
- Runtime path: `ais-runner` -> `ais-engine` -> executors
- Agent/provider path: `ais-runner` + `ais-llm`
