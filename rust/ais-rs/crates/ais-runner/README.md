# `ais-runner`

CLI wrapper for AIS SDK/engine workflows.

## Responsibility

- Provide CLI command skeleton for `run plan`, `run workflow`, `plan diff`, `replay`
- Provide an interactive outer loop via `agent` (multi-round commands + user confirm)
- Keep pause-point decisions behind a single policy interface (`DecisionPolicy`) for predictable `safe|assist|yolo` behavior
- Implement dry-run output (`text` default, `json` optional)
- Load plan/runtime/workspace files and delegate to `ais-sdk` parse/validate/planner APIs
- For `run workflow`, merge `workflow.inputs.*.default` into runtime `inputs` when missing (runtime explicit values take precedence)
- Build runner chain config (`ais-runner/0.0.1`) and assemble exact-chain router executors
- Bridge engine run statuses (`completed|paused|stopped`) to CLI output/rendering
- Persist checkpoint idempotency ledgers by consuming engine `side_effect_observed` events
- Surface engine conditional observability fields (`event.data.checks`) in events/trace/verbose output so failed paths can be traced to `condition/gate/assert` true/false decisions

## Recent changes

- `CB-fix` follow-up: converged hardcoded limits in `budgeter`/`planning_memory`/`tools/dispatch` into `context/budget_policy.rs` single-source policy tables; dynamic projection caps, pressure prune keep-limits, and dispatch compact profiles are now policy-derived.
- `CB-fix` follow-up: tool-memory projection budget now derives dynamic bounds from model soft context limit (`20%~40%` of `context_soft_limit_tokens` when available), with absolute safety clamp `1200..64000`; fallback absolute-remaining mapping is kept when soft limit is missing.
- Added `src/agent/context/CONTEXT_BUDGET_STRATEGY.md` to document the current dynamic context/tool-memory budget design, pressure-mode thresholds, and runtime integration points.
- `CB-fix` follow-up: fixed `PlanningMemory` critical-pressure guide pruning priority to keep high-value guide entries (`ais-plan-sketch`/`cel`) by semantic priority instead of signature lexical order; added targeted regression test.
- `CB-fix` follow-up: wired dynamic `PlanningMemory` store budget derivation from `context/budget_policy.rs` into orchestrator refresh, and switched planner checkpoint/restore insertion paths to use current runtime budget instead of hardcoded defaults.
- `TT-P3-010` follow-up: completed deeper decode/dispatch sink. `decode_segmented_tool_call*` implementation and `PlannerToolOutput/DecodedSegmentedToolCall` type definitions now live in `src/agent/tools/dispatch.rs`, while `intent_segmented.rs` keeps only thin orchestration-facing wrappers.
- `TT-P3-010/020/030` (Wave-TT-E): completed final cleanup/docs/qa closeout; removed remaining redundant cache-key forwarding helper in `intent_segmented`, moved `agent/context/envelope.rs` inline tests to `src/agent/tests/context_envelope.rs`, and closed gate regression set (`orchestrator`/`segmented_*`/`intent_segmented`/`checkpoint_ext` + `cargo check`).
- `TT-P1-040B` (Wave-TT-D): moved `intent_segmented.rs` inline tests to `src/agent/tests/intent_segmented_module.rs`; source now keeps only path-mounted test module declaration.
- `TT-P2-010/020/030/040` (Wave-TT-C): extracted segmented tool-calling policy/normalize/cache/check-segment helpers into `src/agent/tools/{names,phase_policy,decode,guide,cache,check_segment}.rs`; `intent_segmented.rs` now delegates those concerns to `tools/*` modules.
- `TT-P2-040` follow-up compatibility fix: repeated `plan.check_segment` failure payload now preserves upstream `reason_code` when present, and finalize-vs-check segment signature comparison now uses the same normalized segment shape to avoid false mismatch retries.
- `CB-P0-010/020` (Wave-CB-A): froze当前上下文预算/裁剪基线，并新增 `src/agent/context/budget_policy.rs` 单源策略骨架（`context/mod.rs` 已挂载，后续波次将在此接管 `tool_memory_projection` 预算迁移）。
- `CB-P1-010/020/030` (Wave-CB-B): 将 `orchestrator.rs` 与 `planning_memory.rs` 的 `tool_memory_projection` 预算常量收敛到 `context/budget_policy.rs`；`context/budgeter.rs` 压力模式判定复用策略模块实现，`resolve_tool_memory_projection_token_budget` 接管到统一策略函数。
- `CB-P2-010/020/030` (Wave-CB-C): 落地 `prune_for_pressure` 与分层清理策略并在 `refresh_tool_memory_projection` 中接入，支持压力下清理并重排 tool memory 投影。
- `CB-P3-010/020/030` (Wave-CB-D): 增加 `runtime.agent.llm_usage.diagnostics` 可观测字段并闭环回归，包含 `memory_prune_runs` / `memory_pruned_entries_total` / `memory_pruned_by_tool` / `memory_projection_budget_tokens` / `memory_projection_estimated_tokens` / `memory_projection_empty_due_to_pressure_total`。
- `TT-P1-010/020/030/040A` (Wave-TT-B): migrated test implementation bodies out of source modules into `src/agent/tests/**` (including `phase_machine/*`, `orchestrator`, `brain`), with source files keeping minimal `#[cfg(test)] #[path = ...] mod tests;` declarations only.
- `TT-P0-020` (Wave-TT-A): created `agent/tools/` extraction scaffold (`names/phase_policy/decode/dispatch/cache/guide/catalog/check_segment`) and mounted `mod tools;` in `agent/mod.rs`.
- `TT-P0-020` (Wave-TT-A): test entry scaffold now has `src/agent/tests/mod.rs`; `mod_test.rs` includes this single aggregator file as migration landing point.
- `SS-P5-040`: single-source input model closeout completed. `InputStore` is the only input semantic source in runner main-flow and checkpoint payloads; `runtime_store` now only contains minimal runtime agent field writers.
- `SS-P5-020`: planning context projection signatures now consume `InputStore` directly (`context_view` / `context.projector` / `context.collector`), and orchestrator `state_summary` refresh reads `InputStore` as the single input source.
- `SS-P5-020`: grounding tests now assert canonical `InputStore` key semantics (`owner` and `inputs.owner` resolve to one canonical slot) to match strict single-source behavior.
- `SS-P5-020`: write-gate validation now reads `InputStore` directly (including volatile freshness via `InputStore` metadata `stability/source/observed_at_ms`) instead of `FactStore`.
- `SS-P5-020`: checkpoint extension encoder now accepts `InputStore` directly (`encode_updated`), removing semantic-to-input reconversion from the encode path.
- `SS-P5-030`: removed `InputSemanticStore` from `src/agent/**`; orchestrator/phase-machine/checkpoint/missing-input signatures now use `InputStore` directly with no semantic-store alias.
- `SS-P5-030`: removed all `runtime_store` projection/FactStore helpers; module now keeps only minimal runtime-agent field writers (`record_runtime_agent_field` / `record_todo_progress`).

## Public entry points

- Binary: `ais-runner`
- Config APIs:
  - `load_runner_config(path)`
  - `validate_runner_config(config)`
  - `build_router_executor(config)`
  - `build_router_executor_for_plan(plan, config)`
- Commands:
  - `ais-runner run plan --plan <file> [--config <runner-config>] [--runtime <file>] [--dry-run] [--events-jsonl <path|->] [--trace <path>] [--checkpoint <path>] [--commands-stdin-jsonl] [--verbose] [--format text|json]`
  - `ais-runner run workflow --workflow <file> [--workspace <dir>] [--config <runner-config>] [--runtime <file>] [--dry-run] [--events-jsonl <path|->] [--trace <path>] [--checkpoint <path>] [--outputs <json-file>] [--commands-stdin-jsonl] [--verbose] [--format text|json]`
  - `ais-runner plan diff --before <plan> --after <plan> [--format text|json]`
  - `ais-runner replay [--trace-jsonl <file>] [--checkpoint <file> --plan <plan> --config <runner-config>] [--until-node <id>] [--format text|json]`
- `ais-runner agent (--plan <file> | --intent <text> | --intent-file <file>) --config <runner-config> [--workspace <dir>] [--pack <pack-file>] [--runtime <file>] [--events-jsonl <path|->] [--trace <path>] [--checkpoint <path>] [--profile standard|demo-scripted] [--llm-script-jsonl <file>] [--verbose] [--verbose-llm] [--approvals-mode safe|assist|yolo] [--max-iterations <n>] [--max-planner-rounds <n>] [--max-tool-rounds <n>] [--max-index-candidates <n>] [--planner-context-token-budget <n>] [--format text|json]`

## Intent mode quick guide

`agent --intent|--intent-file` supports a full loop:
intent parsing → LLM tool-calling planning → execution → pause/confirm → optional repair.

Intent mode requires `--workspace` candidates and uses the vNext segmented loop:

- `plan.begin` → `plan.ground_intent` → `plan.propose_todos` → `plan.propose_segment` / `plan.revise_segment`
- candidate discovery/check tools: `list_candidates` / `catalog.search` / `get_candidate_detail` / `guide.get` / `plan.check_segment`
- `list_candidates` returns protocol-grouped discovery snapshots (`protocol/chains/actions[]/queries[]`) plus `execution_plugins`; each action/query card is compact (`ref/chains/required_inputs`) to minimize token overhead. It accepts optional filters (either top-level fields or `filter` object): `chain` (exact `<namespace>:<id>` or namespace wildcard `<namespace>:*`) and `protocol` (case-insensitive contains). `catalog.search` returns ref-first compact matches (`ref/kind/chains?/risk_level?`) for targeted filtering; call `get_candidate_detail` for full params/returns/risk/execution details
- segmented planner prompt uses one shared abstract `list_candidates` filter-first policy template in base rules, and phase prompts only reference that template: start exact `chain`, add hinted `protocol`, broaden only on empty/insufficient results in order `exact chain+protocol -> exact chain -> chain namespace wildcard`.
- segmented planner prompt rules are slimmed and deduplicated: shared discovery/safety contracts live in base rules, while phase prompts keep only phase-specific deltas (tool allowlist, phase goal, finalize/check requirements).
- `capability_view` is semantic-ready (`ais-agent-capability-view/0.0.2`): per protocol it exposes `topics[]` + `topic_cards[]` in addition to raw `actions/queries/required_inputs`; topic values are declaration-driven (`extensions.agent.topic(s)` / `risk_tags` / `meta.tags`) and no longer inferred from action/query name substrings
- planner rounds enforce strict tool-call FSM: begin round only `plan.begin`; propose/revise rounds only discovery tools + matching finalize tool, with finalize at most once and always last
- planner rounds now include `ground_intent` phase: extract initial `resolved_inputs/intent_facts` with confidence before todo planning; low-confidence fields are returned as questions
- grounding decode hardening: if model returns `status=proposed` but omits `ready_for_todos`, runner infers readiness only when no questions are present and resolved inputs are non-empty (prevents false `missing_required_input` pauses on well-grounded payloads)
- `AGT-LI-011` (ground-intent not-ready actionability hardening): for `plan.ground_intent`, `status=proposed` + `ready_for_todos=false` now requires actionable follow-up (`questions` or `missing_refs`, non-empty). Non-actionable payloads are rejected by finalize decode, emit schema-repair hints with explicit good/bad examples, and participate in bounded in-round repair retries.
- `AGT-LI-010` (grounding non-actionable pause guard): if grounding yields `ready_for_todos=false` but pauses with empty `questions` and empty `missing_refs`, orchestrator now detects this deadlock pattern, emits `grounding_non_actionable_pause_detected`, performs one bounded grounding repair retry (`grounding_repair_retry`), and on retry exhaustion emits an actionable unavailable fallback (`reason_code=grounding_non_actionable_pause`) with non-empty `questions/missing_refs` instead of leaving a non-actionable `missing_required_input` pause.
- grounding decode tolerates stringified JSON fields for `resolved_inputs` / `intent_facts` / `confidence` / `questions` / `issues` (including provider glitches that nest full grounding payload JSON under `intent_facts`)
- grounding planner-call transport/runtime failures now take an explicit fallback contract: runner records `intent_grounding.status=fallback` with `reason_code=planner_call_failed`, keeps `ready_for_todos=true`, preserves the error in `previous_error` context, and continues into todo planning instead of hard-failing the run.
- grounding apply path now canonicalizes resolved input keys (`inputs.owner`/`runtime.inputs.owner` -> `owner`) and unwraps wrapper values (`{"value":..., "confidence":...}`), preventing runtime key drift like `/inputs/inputs/*` that can trigger false `missing_required_input` pauses; key normalization/runtime-input writes are centralized in `agent/input_normalize.rs` and reused by both agent answer backfill and orchestrator grounding apply.
- deterministic grounding fallback (`AGT-LI-012`) extracts `inputs.balance_threshold` from high-confidence `balance > N` patterns found in intent text / grounding facts (for example `native_balance > 100 AND tst_balance > 100`), writes provenance `rule_extracted.balance_threshold`, and records runtime observability fields under `runtime.agent.intent_grounding.{deterministic_rule_inputs,deterministic_rule_skipped,deterministic_conflicts}`. Conflict policy is explicit and stable: `rule_extracted_over_llm`.
- finalize tool input schemas are sourced from embedded `ais-agent-planning-tools/0.1.0` definitions (`segment_payload`), avoiding runner-local schema drift
- finalize tool-call contracts are conditional:
  - `plan.propose_segment` / `plan.revise_segment` with `status=proposed` MUST include a `segment`
  - with `status=invalid|unavailable` MUST include an `error.reason_code`
- segmented step kinds support `query|action|assert|branch`
- `plan.begin.cursor` accepts string/number and is normalized to string for session state (`"0"` style cursor is valid)
- `plan.begin` prompt payload now includes host-derived `snapshot_hash`; planner must echo this exact hash to avoid begin-session drift
- planner tool-call decoding accepts `segment` as object or JSON-stringified object text; non-object/invalid JSON still yields deterministic repair reason codes
  - retry/error classification maps `proposed segment draft \`segment\` must decode to a JSON object` to planner `sub_reason_code=segment_not_json`, so revise loops treat this output as retryable planner-format failure
- `AGT-P4-050` (planner schema hardening): when `plan.propose_segment`/`plan.revise_segment` finalize args miss required `status`, runner no longer fails fast into outer planning-loop exhaustion; it injects an in-round schema-repair tool payload (`reason_code=schema_missing_required_field`, `sub_reason_code=missing_status`) with bounded attempts, then classifies remaining failures as planner invalid-tool-output for revise routing.
- `AGT-LI-008` (planner schema/type graceful degrade): repairable finalize decode errors now include invalid JSON type mismatches (for example string `"false"` for boolean `done`) in addition to missing required fields; runner injects bounded in-round repair payloads, emits repair retry/exhausted trace events, and records diagnostics counters under `runtime.agent.llm_usage.diagnostics.finalize_schema_repair_*` while preserving strict failure for non-repairable planner errors.
- planner finalize decode core now uses typed serde adapters for `issues/questions/error/details` (segment/todo/grounding) with raw-value fallback; tool boundaries and runtime draft surfaces remain JSON `Value` compatible
- segment decode applies shape guardrails: missing `steps[*].inputs` is auto-filled as `{}` before schema decode, reducing retry loops from minor tool-output omissions
- `get_candidate_detail` keeps minimal callable signatures stable for LLM planning (`params[].name/type/required`, `returns[].name/type`) while still applying payload budget compaction
  - propose/revise prompts include explicit contracts for step fields, ValueRef (`lit/ref/cel/object/array`), CEL namespaces, and dependency references
  - propose/revise prompt contracts explicitly forbid legacy branch-tree step fields (`if_true/if_false/then/else/children`) and require branch paths to be encoded as flat steps with `when.cel + depends_on`
- segmented planner `state_summary` now carries a structured `input_store` payload (`facts` + `meta` with source/provenance), so model-side planning can consume runtime/config-derived facts deterministically
- segmented planner `state_summary` also carries `input_slots` (`resolved`/`missing`/`canonical_refs`) so model output can anchor to stable `inputs.*` refs instead of guessing input paths
- segmented planner `state_summary` now carries `input_registry` (`known_refs` + entry metadata) and planner prompts require `inputs.*` refs to come from this registry
- `SS-P2-010` (intent context split): `state_summary.intent_slots` is now input-only (`resolved_inputs` / `resolved_input_refs` / `confidence.inputs` / `input_binding`), while non-input intent semantics live in `state_summary.intent_context` (`facts`, `confidence.facts`, grounding status/questions/reasons). This removes legacy mixed keyspace behavior where semantic facts were carried under input-slot paths.
- segmented planner `state_summary` now carries chain-agnostic `canonical_context` (`chain_refs/account_refs/asset_refs/amount_refs`) so planning can reason across EVM/Solana-style account/asset shapes without EVM-only field assumptions
- segmented planner `state_summary` now carries compact `node_output_refs` (`entries[].step_id + refs[]`, plus `known_refs[]`) so planner can reuse concrete `nodes.<step_id>.outputs.*` references from observed runtime outputs with lower token cost
- segmented planner `state_summary` now carries `tool_memory_projection` (bounded recent memory for `list_candidates` / `catalog.search` / `get_candidate_detail` / `guide.get`) so planner can reuse high-value discovery/schema context before issuing duplicate tool calls
  - projection now includes `recent.list_inventory[]` (protocol-grouped inventory summaries) to avoid repeated broad discovery in propose/revise phases
  - projection applies stronger dedupe (cross-entry `ref` dedupe and repeated schema/topic collapse) and guide priority ranking (`ais-plan-sketch` / `cel` first) to keep high-signal context within token budget
  - guide memory is structured as keyed maps (`recent.guide.schema.<schema_id>` / `recent.guide.topic.<topic_id>`), and schema `full` payloads replace prior digest entries for the same schema id
  - projection token budget is adaptive (`1200~6000`): derived from current context headroom (`context_remaining_tokens/context_soft_limit_tokens`) when available, otherwise by absolute remaining-token fallback
- segmented planner now tracks loop/efficiency diagnostics in `runtime.agent.llm_usage.diagnostics`: `duplicate_tool_call_ratio`, `discovery_tool_call_ratio`, `empty_search_streak_max`, `memory_hit_rate_by_tool`, `phase_round_count`
- 压力触发 `tool_memory` 投影刷新时追加内存压缩诊断：
  - `memory_prune_runs`
  - `memory_pruned_entries_total`
  - `memory_pruned_by_tool`
  - `memory_projection_budget_tokens`
  - `memory_projection_estimated_tokens`
  - `memory_projection_empty_due_to_pressure_total`
- segmented planner includes a phase-local loop guard for repeated empty `catalog.search`; when the same empty search pattern repeats, runner injects structured guidance to pivot to memory/list/detail/guide tools instead of continuing empty discovery (hint is emitted once per streak threshold hit, not every round)
- when `plan.check_segment` reports `candidate not found for control step`, runner injects a targeted repair hint (avoid searching synthetic `.../assert` refs; use discovered refs + `when/depends_on` gating) to reduce revise-loop thrashing
- segmented runner also maintains `runtime.agent.todo_progress` and injects `state_summary.todo_state` (`current_todo` + progress counters), so each planning round is scoped to one explicit todo objective
- `context_unchanged=true` now keeps the full projected `state_summary` payload (marker only), so cross-phase planning does not lose registry/fact/tool-memory context.
- context projection now emits a versioned `context_envelope` contract (`schema/schema_version/version/hash/unchanged`) and keeps legacy top-level `context_version/context_hash/context_unchanged` for planner/orchestrator compatibility.
- initial `input_store` seeds flattened runtime `inputs` into both `<slot>` and canonical `inputs.<slot>` keys (for example `owner` + `inputs.owner`, `token.address` + `inputs.token.address`) in addition to runtime fallback owner/wallet and signer-derived addresses (`owner_by_chain.<chain>`), with priority order `user > query > config > runtime > derived > intent`
- `guide.get` request shape is strict and string-only: `{schema:"ais-plan-sketch/0.1.0"}` or `{topic:"cel"}`; object/nested compatibility shapes are rejected
  - schema responses default to compact `digest` mode (`schema.digest`) instead of returning the full schema JSON; use `{schema:"...",full:true}` only when full schema payload is strictly required
  - planner decode path applies a narrow args normalization before validation/dispatch for `guide.get.full` only (`"true"/"false"` string -> boolean), and emits `tool_args_normalized` trace telemetry when this repair is applied
  - cache semantics are canonical by schema/topic id: when `{full:true}` is requested after a cached digest, runner refreshes and replaces the cached entry with full schema payload instead of keeping duplicate digest/full copies
- segmented planner prompt now reinforces strict tool-argument typing (no quoted booleans/numbers for schema bool/number fields), adds concise positive/negative examples (`guide.get.full`, numeric limits, finalize `done`), and includes a pre-tool/finalize self-check checklist for phase gating, required fields, and exact JSON type conformance
- `plan.check_segment` runs compile-only segment validation and returns structured issues (`ok=false` + `issues[]`) without mutating active plan or executing nodes
- write-gate failures from `plan.check_segment` now include actionable fields (`gate_reason_code`, `action_depends_on`, `gate_step_ids`, `gates_missing_query_dep`) so revise loops can patch exact dependency links
- runner enforces a successful `plan.check_segment` (`ok=true`) before accepting `plan.propose_segment`/`plan.revise_segment` outputs with `status=proposed` (unavailable/invalid drafts are exempt from this gate)
- finalize-stage guard now binds to checked draft signature: if proposed segment differs from the last `check_segment(ok=true)` draft, runner blocks finalize and forces re-check on the updated segment
- revise/propose rounds now short-circuit when `plan.check_segment` returns the same failure signature repeatedly (default threshold: `3`), returning structured repair guidance instead of burning all tool rounds
- `AGT-LI-013` input-ref compile-entry guard now runs before both `plan.check_segment` and execution compile: `inputs.*` refs are enforced against `state_summary.input_registry.known_refs`, safe aliases are canonicalized (`runtime.inputs.*`, `input.*`, `*.value`, bounded separator normalization), and illegal refs return structured `unknown_input_ref` issues with ranked candidates
  - `AGT-LI-015` unknown-input auto-repair suggestions are now staged and deterministic: candidate ranking prioritizes exact canonical alias matches first, then grounding-aware deterministic alias candidates (including `intent_slots.intent_facts` hints such as `fact:token` -> canonical token refs), and only then bounded semantic fallback; compile-autofill emits trace event `unknown_input_ref_repair_suggested` with top candidates.
  - `AGT-LI-019` prompt semantic guard for unknown-input repair: token/address params prefer address-like refs (for example `*.address`), and `*.decimals` refs are explicitly disallowed as substitutes for token/address slots.
- segmented draft step contract is conditional: each `segment.steps[]` must include `id/kind/inputs`; `candidate_ref` is required for `query|action` and optional for built-in `assert|branch`
  - when planner omits `candidate_ref` on executable steps (`query|action`), runner emits targeted diagnostics with missing step ids/kinds and classifies retry reason as `missing_candidate_ref` for deterministic repair prompts
  - planner-output repair payload now includes `previous_error.last_failed_finalize` (failed finalize tool call args + assistant snippet, compacted), so revise rounds can patch the previous draft instead of regenerating from scratch
- `segment.steps[].depends_on` may only reference step ids in the same segment (cross-segment ids like `seg_1/...` are invalid)
- planner missing-input contract: when required facts are missing, planner should return `status=unavailable` + `error.reason_code=missing_required_input` + `error.details.questions[]`
- runner treats `missing_required_input` as a pause (not hard failure) and records machine-readable payload at `runtime.agent.missing_required_input`
- execution-stage `need_user_input` pauses with `reason_code=missing_required_input` are now normalized into the same payload contract (`missing_refs/suggested_paths/questions`), so缺参不会落入 `need_user_confirm`.
- missing-input orchestration glue is modularized in `agent/missing_input.rs` (payload assembly, pause marking, optional interactive answer collect/apply backfill), and reused across grounding/todo/plan/execute-pause paths for consistent behavior.
- missing-input question handling now supports pre-interactive auto selection of a single query-intent option (for example `Query ...`), so query-capable required fields can continue planner flow without forcing manual stdin confirmation; unresolved/ambiguous cases still fall back to interactive questions.
- host-side missing-input autofill now closes the query-option loop for grounding/todo/segment-unavailable/execute-pause paths: when payload questions include query-intent options, runner resolves missing refs via `catalog.resolve_missing_facts` and injects `autofill.selected_query_refs` into planner previous-error context for an automatic retry before pausing for manual input.
- runtime `/agent/*` write path is centralized in `agent/runtime_store.rs` (`record_runtime_agent_field` / `record_todo_progress` / `record_missing_required_input`), reducing duplicated JSON mutation logic across orchestrator and helpers.
- phase-machine core types are introduced in `agent/phase_machine/types.rs` (`AgentPhase` / `PhaseTransition`) and cover baseline states `Init/GroundIntent/PlanTodos/PlanSegment/ExecuteSegment/ResolvePause/Completed/Failed` for subsequent orchestrator migration.
- phase-machine main-flow runner lives in `agent/phase_machine/mod.rs`; segmented orchestrator entry now routes through `phase_machine::run_main_flow(...)` as the single production entry (legacy bridge fallback removed).
- phase-machine main-flow now receives explicit orchestrator phase hints (`grounding/todo/plan/execute/pause`), records transition telemetry (`advance|stay`), and attributes terminal failures to the last reported active phase instead of defaulting to `ground_intent`.
- GroundIntent phase handling is migrated into `agent/phase_machine/grounding.rs`; orchestrator now delegates grounding bootstrap/proposed-unavailable-invalid handling to this module while preserving runtime/fact-store side effects and pause semantics.
- TodoPlanning bootstrap/fallback handling is migrated into `agent/phase_machine/todo.rs`; orchestrator now routes todo proposed/unavailable/invalid and `missing_required_input` pause paths through the phase-machine module without changing grounding or execute-round behavior.
- SegmentPlanning propose/revise + planner-output retry handling is migrated into `agent/phase_machine/segment_plan.rs`; orchestrator now routes planning rounds to this module while preserving previous-error refresh and retry semantics.
- ExecuteSegment replace/execute/checkpoint lifecycle is migrated into `agent/phase_machine/segment_exec.rs`; orchestrator now routes segment plan replacement, run-loop callbacks, event todo/segment annotations, runtime store-to-fact-store mapping, and todo receipt status shaping through this module with parity behavior.
- ResolvePause backflow is migrated into `agent/phase_machine/pause.rs`; orchestrator now routes pause classification/resolution through one entry that cleanly splits `missing_required_input` vs `need_user_confirm` while preserving pause trace/event payload contracts.
- `SS-P1-010` removes business-side direct `runtime.inputs.*` owner/wallet reads in initial owner seeding; missing-input checks, compile known refs, and pause backflow now rely on InputStore semantics carried by `input_store` (`inputs.*` + canonical aliases), while `runtime.inputs` remains projection/state payload only.
- on interactive TTY runs, runner prompts user to answer `questions[]` (choose option index or enter custom JSON/string), backfills `runtime.inputs.*` + planning `input_store`, and immediately retries planning via `plan.revise_segment`
- segmented orchestrator also handles execution-time missing-input pauses by prompting/回填/自动继续; unresolved answers keep pause as `missing_required_input` and block current todo deterministically.
- todo-first loop is host-enforced: each round advances exactly one current todo (`todo -> in_progress -> done|blocked`), and non-final rounds auto-open follow-up todo entries
  - `AGT-LI-007` completion gate: when the current todo is completed, no todo remains, and declared todo acceptance facts are already satisfied in `input_store`, runner exits as completed instead of creating placeholder follow-up todos like `Continue intent segment N`.
- host-side write gate validation blocks transfer/swap segments without a query-backed control gate path (`query -> ... -> assert|branch -> ... -> action`, recursive across segment-local deps); query endpoints in this chain must resolve to discovered query candidates. It also enforces token-decimals availability for asset writes
- compile segment (`plan-sketch` IR) to executable `ais-plan`
- append via guarded `replace_plan`
- execute + checkpoint, then continue next segment

### Demo-scripted profile (deterministic)

Use fixture `rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer`:
commands below assume working directory `/home/xcshuan/work/owlia/ais`.

```bash
cargo run --manifest-path rust/ais-rs/Cargo.toml -p ais-runner -- agent \
  --intent-file rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.success.jsonl \
  --approvals-mode safe \
  --format text
```

### Standard profile (real provider)

Use one template under `rust/ais-rs/fixtures/runner-local/llm-providers/config/` and set env keys:

```bash
cargo run --manifest-path rust/ais-rs/Cargo.toml -p ais-runner -- agent \
  --intent "check balances then transfer if both >100" \
  --workspace rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config rust/ais-rs/fixtures/runner-local/llm-providers/config/openrouter.config.yaml \
  --profile standard \
  --approvals-mode safe \
  --format text
```

`runner config llm` supports provider chain controls:

- `fallback[]`: backup providers (`provider/model/api_key/api_base`)
- `max_retries_per_provider`: retry budget per provider attempt (default `1`)
- `rotation: sticky_primary|round_robin`
- `prompts_dir`: optional markdown prompt directory for runtime overrides (loaded dynamically on each prompt render; fallback to built-in prompts on missing/invalid files)
- `planner_context_token_budget`: optional `state_summary` token budget override for segmented planner context projection (default `6000`, CLI `--planner-context-token-budget` has higher priority)
- `max_tool_rounds`: optional max LLM tool rounds per segmented planner phase call (`ground_intent` / `propose_todos` / `propose_segment` / `revise_segment`), default `24`, CLI `--max-tool-rounds` has higher priority
- `context_limit_tokens`: optional LLM context limit for usage tracking (supports integer or human-readable string like `262k` / `1M` / `262,144`); runner computes remaining headroom against a 90% soft limit.
- planner context projection now uses adaptive budgeting: when `runtime.agent.llm_usage.context_remaining_tokens` ratio is high, `state_summary` budget is relaxed (less aggressive trimming); when low, it tightens automatically.
- `state_summary` projection is emitted directly from context-budget + pressure strategy output (no extra post-pass compact layer), so high-value registry/context fields remain visible when budget allows.
  - `context_remaining_tokens` now means per-call context-window headroom (soft limit - current request input tokens).
  - compression is near-window driven: `<70%` usage keeps full context, `70~85%` light compaction, `85~92%` medium compaction, `>92%` critical compaction.
  - pressure classification uses worst-case signal selection: when both `context_window_input_tokens` usage ratio and `context_remaining_tokens` are available, runner picks the more conservative pressure; absolute remaining-token guards (`<=8000` tight, `<=3000` critical) are always enforced.
  - critical pressure applies structural shedding: trim duplicate projections first (for example `input_slots.canonical_refs`), drop low-priority heavy sections (`capability_view.protocols`), and aggressively compact large blobs (`tool_memory_projection`, `previous_error.last_failed_finalize`).

Prompt override file names under `prompts_dir`:

- `agent.controller.system.md`
- `segmented.base_rules.md`
- `segmented.contracts_summary.md`
- `segmented.phase.begin.md`
- `segmented.phase.grounding.md`
- `segmented.phase.todos.md`
- `segmented.phase.propose.md`
- `segmented.phase.revise.md`
- `segmented.begin.patch.md` (JSON object patch, deep-merged into begin prompt payload)
- `segmented.grounding.patch.md` (JSON object patch, deep-merged into grounding prompt payload)
- `segmented.todos.patch.md` (JSON object patch, deep-merged into todo planning prompt payload)
- `segmented.segment.patch.md` (JSON object patch, deep-merged into propose/revise prompt payload)

Prompt file format:

- plain markdown body, or markdown with optional frontmatter (`--- ... ---`), body used as prompt.
- for list-based segmented prompts, each non-empty line becomes one rule; markdown bullets/numbered list prefixes are normalized.
- segmented prompt rule convergence guardrails:
  - `candidate_ref`: required for `query`/`action`; optional for `assert`/`branch` control steps.
  - phase tool allowlists for `ground_intent` / `propose_todos` / `propose_segment` / `revise_segment` include `catalog.resolve_missing_facts`.

Prompt source of truth and fixture sync:

- Runtime source of truth lives in `src/agent/intent_segmented.rs`: `SegmentedPromptContextBuilder::default()` (default rule text) and `ensure_tool_allowed_for_phase(...)` (enforced phase allowlists).
- Fixture prompts under `rust/ais-rs/fixtures/runner-local/llm-prompts/prompts/` are integration mirrors for local prompt overrides and must preserve key anchors:
  - base-rule `candidate_ref` semantics (`query`/`action` required; `assert`/`branch` optional)
  - shared `list_candidates` filter-first policy template anchor
  - phase `Allowed tools` anchors for `ground_intent` / `propose_todos` / `propose_segment` / `revise_segment`
- Drift guard test: `fixture_prompt_rules_align_with_runtime_prompt_sources_of_truth` (in `src/agent/intent_segmented.rs`) checks those anchors against runtime defaults/allowlists.
- Update workflow when changing prompt defaults or allowlists:
  - update runtime defaults/enforcement in `intent_segmented.rs`
  - update mirrored fixture markdown under `fixtures/runner-local/llm-prompts/prompts/`
  - run `cargo test --manifest-path rust/ais-rs/Cargo.toml -p ais-runner intent_segmented -- --nocapture`
  - only merge after the drift guard test passes

### Approvals and safety

- `safe`: always prompt on `need_user_confirm`.
- `assist`: LLM may auto-approve only within pack risk threshold.
- `yolo`: auto-approve confirmation nodes; do not use with real funds.
- Manual confirm prompt supports `approve|deny|always_approve_this_run|cancel`.
- Prefer local test chains; demo private keys in fixtures are for local-only usage.
- LLM tool-result payloads are sanitized by default before being fed back into planner rounds (sensitive key redaction + prompt-injection-like text neutralization).
- Planner/tool context uses compact JSON budgeting by default (depth/object/array/string limits), and `get_candidate_detail` enforces a bounded refs window to keep token usage predictable on large catalogs.

### Verbose logging

- `--verbose`: prints engine event stream + policy gate output details + checkpoint load/save hash/epoch details.
- `--verbose-llm`: prints planner tool definitions at startup (name + input schema), system/user prompts, assistant content per round, tool-calls (`tool_call_id` + tool name + args), candidate tool result summaries + returned tool prompts, repair payloads/diffs (`plan.revise_segment`), and per-call context usage metrics (`input_tokens/output_tokens/total_tokens`, cumulative input/output split, and current-window headroom `context_remaining_tokens` when `llm.context_limit_tokens` is configured).
- `--verbose-llm` also prints per-round planner summary (`phase/pressure_mode/compressed/memory_hits/duplicate_ratio_bps/empty_search_streak_max`) and duplicate tool-call diagnostics, so discovery loops can be identified directly from one log.
- segmented orchestrator now logs planning attempt mode and previous error compactly (`phase/reason/sub_reason`), plus compile guard compact failure summaries (reason + first issue), so repeated `revise_segment` restarts are attributable from logs.
- segmented orchestrator now includes a lightweight tracing helper (`agent/trace.rs`) that emits unified lines (`[agent.trace] phase=<...> event=<...> ...`) for key flow phases: `grounding`, `todo`, `plan_round`, `execute_round`, `pause_resolution` (enabled under `--verbose` or `--verbose-llm`).
- checkpoint save orchestration for segmented loop is extracted to `agent/checkpoint_flow.rs` (`checkpoint_round` + planning-failure event+checkpoint), reducing duplicated checkpoint wiring across orchestrator branches.
- planning-failure checkpoint recording is best-effort in orchestrator error paths: if failure-event checkpoint persistence fails, runner preserves and returns the original planning error, and logs checkpoint persistence failure context under verbose modes.
- segmented terminal-state checkpoint consistency: when a segmented run exits with `completed|stopped`, orchestrator writes one final checkpoint after todo-board terminal transitions, so checkpoint `runtime_snapshot.agent.todo_progress` stays aligned with CLI terminal summary (no stale `current_todo=in_progress`).
- agent run now pre-initializes `--events-jsonl`/`--trace` output sinks at startup, so paths are materialized even when execution stops before the first engine event is emitted.
- Checkpoint persistence now includes `approvals_ledger` + `side_effects` (tx hash/idempotency key); on resume, runner marks only `confirmed` side-effects as completed and asks chain executors to reconcile `sent` side-effects before continuing (pending reconcile pauses run to avoid blind replay; reverted reconcile statuses pause with `side_effect_reconcile_reverted:*`).
- Resume path rehydrates approved confirmation nodes from checkpoint `approvals_ledger` (excluding already completed nodes), so `need_user_confirm -> resume` remains idempotent and does not re-prompt/replay already approved actions.
- Resume dedupe for confirmed writes now matches `node_id + confirmation_hash + confirmed side_effect` before short-circuiting execution; resumed runs emit trace markers `side_effect_reused` / `resume_skip_confirmed_write` and do not re-submit tx for the same confirmed confirmation intent.
- Checkpoint side-effect ledger keeps at most one `confirmed` record per `node_id` (different idempotency keys for the same node are ignored once a confirmed record exists), preventing duplicate confirmed side-effect finalization during restore/reconcile flows.
- runner records normalized side-effect lifecycle summary at `runtime.agent.side_effect_lifecycle` (`sent/confirmed/reverted` counters + per execution type breakdown), and `state_summary` projects this structure for segmented planning context.
- segmented checkpoint extensions persist `input_store` + `todo_progress` (with `planning_memory`) so resumed runs keep planning context and previously supplied facts.
- segmented checkpoint extensions also persist typed `intent_facts`; restore path injects them back into `runtime.agent.intent_grounding.intent_facts` and merges into planning `input_store`.
- `input_store` overwrite guard keeps intent semantic facts stable against volatile query observations (for example balance/allowance refresh values do not rewrite intent constants).
- segmented planner `state_summary` now applies staged context-budget projection (`balanced/tight/minimal`) with stable clipping order; key slots (`owner/wallet/token/amount/chain`) are preserved first and `context_budget` metadata is exposed to the model.
  - `context_budget.token_limit_scope=payload_core`: compaction stage selection/truncation is evaluated on payload core tokens (`estimated_payload_core_tokens`, legacy alias `estimated_tokens`).
  - `context_budget` now also exposes payload-vs-emitted estimates: `estimated_payload_tokens` (payload including `context_budget` block) and `estimated_emitted_tokens` (final emitted summary including `context_envelope` + legacy compatibility fields), plus explicit metadata overhead deltas.
- context projection internals are modularized under `agent/context/` (`collector`/`projector`/`budgeter`), with `agent/context_view.rs` kept as a thin compatibility/orchestration facade for existing callers.
- context contract logic is centralized in `agent/context/envelope.rs`, including schema/schema_version validation on envelope reads, optional payload-hash verification, legacy-summary fallback compatibility, and payload extraction helpers.
- segmented compile-guard maps `unknown_input_ref` and write-gate missing-fact issues into normalized `missing_required_input` payloads (`missing_refs/suggested_paths/questions`) for a single pause contract.
- `AGT-P4-111`: before pausing on compile-time missing input, host now enforces one controlled compile autofill round: `missing_refs -> catalog-style missing-fact resolution -> selected query refs -> forced revise_segment retry`.
- compile autofill is bounded (at most one host autofill round per todo); unresolved refs or failed query-candidate resolution downgrade to `missing_required_input` pause.
- compile autofill emits explicit trace/runtime markers (`compile_autofill start/resolved/unresolved` + unresolved `reason`) for debugging and checkpoint inspection.
- missing-ref normalization is generic (namespace-aware) and no longer depends on token-specific hardcoded branches.
- `AGT-P4-113` (`tests/docs guardrails`): regression tests now explicitly cover non-token object input slots (for example `recipient.profile`) through the same `missing_required_input` missing-ref projection and answer backfill flow used by `token.*` slots, including checkpoint payload persistence checks.
- agent final output (`text`/`json`) includes session-level `llm_usage` totals for segmented planning runs.
- `AGT-P3-000`（单一输入真源冻结）: 在 `agent/mod.rs` 引入 `AIS_RUNNER_INPUT_STORE_MIGRATION_MODE` 与迁移模式行为约束，作为后续 P3 落地基线；详见 `docs/design-ais-rs-agent-input-store-single-source.md`。
  - 支持模式：`legacy` / `shadow_writes` / `read_through` / `single_source` / `enforced_single_source`（默认 `single_source`）。
- `AGT-P4-112`: query `stores` 命中输入语义槽位时统一 canonical leaf 回填到 `inputs.<leaf>`（不做字段特判），并保持 `FactStore` 元信息（`Observed + QueryObserved + provenance`）；`runtime.inputs` 投影与 checkpoint roundtrip 通过同一输入语义读取路径恢复这些值。

### Warning hygiene

- 收敛策略：优先删除未使用 helper，或将仅测试使用的入口改为 `#[cfg(test)]`；避免长期保留宽泛 `#[allow(dead_code)]`。
- 当前状态（`AGT-P4-020 / Wave-P4-A`）：`agent/input_store.rs` 已删除未使用 `InputRef::canonical_segments`；`agent/mod.rs` 已移除迁移模式枚举的 `#[allow(dead_code)]`，并将仅测试调用的 `compile_segment_plan` 限定为测试构建。
- `agent/context/envelope.rs` 现状：生产暴露面保持最小，兼容/校验辅助读取函数继续维持 `#[cfg(test)]`。

## Dependencies

- `ais-sdk`: parse + dry-run planner APIs
- `ais-llm`: provider-agnostic LLM tool-calling types + provider registry/factory integration (`LlmBrain` + `build_provider`)
- `ais-offchain-executor`: offchain `offchain_apy_query` plugin handler registration
- `clap`: CLI parsing
- `serde_json`, `serde_yaml`: runtime file decoding
- `thiserror`: CLI/domain errors

## Current status

- Implemented:
  - `QA-guard Wave-1/Wave-2/Wave-3/Wave-4` regression pack: `agent::tests::` includes focused assertions for `P1-121` todo-phase payload/progress, `P2-200` `state_summary` projection contracts (`input_registry`/`node_output_refs`), `P2-220-prep` planner/compile sub-reason stability, Wave-2 guards for `P1-122` segment-planning phase parity and `P2-210` context-envelope compatibility (`context_version/context_hash/context_unchanged`), Wave-3 guards for `AGT-P1-123` execute-phase transition parity and `AGT-P2-220-main` typed context-core path parity, plus Wave-4 guards for `AGT-P1-124` ResolvePause backflow split (`need_user_input` vs `need_user_confirm`) and `AGT-P2-230` reason/sub_reason enum compatibility; runnable via `bash rust/ais-rs/scripts/agent_regression_baseline.sh --group wave1|wave2|wave3|wave4`.
  - `AISRS-RUN-001` (CLI 命令骨架 + `--help` smoke test)
  - `AISRS-RUN-002` (workspace 目录加载与分类：protocol/pack/workflow/plan，含 issues 输出)
  - `AISRS-RUN-003` (runner config 解析/校验 + EVM/Solana executor 装配 + plan chain 缺失校验)
  - `AISRS-RUN-010` (run plan dry-run text/json, includes `main.rs` CLI dispatch and `run_test.rs`)
  - `AISRS-RUN-011` (run plan execute loop + events-jsonl sink + trace sink + checkpoint save/restore)
  - `AGT-P4-100` (events/trace realtime timestamp): runner event sinks now consume engine-emitted wall-clock UTC RFC3339 `ts` values (no fixed epoch placeholder), and tests assert timestamp shape/non-epoch semantics instead of hardcoded literal timestamps.
  - `AGT-LI-006` (replace-plan realtime timestamps): non-replay replace-plan command processing now stamps `command_accepted`/`plan_replaced`/replace rejection `error` events with wall-clock RFC3339 timestamps (no epoch defaults); segmented-agent planning-failure `error` emission and approvals-ledger `decided_at` updates in live runs also use wall-clock timestamps.
  - `AISRS-RUN-012` (optional stdin JSONL command ingestion, supports apply_patches/user_confirm/user_input/user_select/cancel, emits command accepted/rejected events)
  - `AISRS-CMD-001` (`replace_plan` command integration): runner pre-processes `replace_plan` commands before each engine step, enforces completed-node mutation guards, performs diff-based re-confirmation (`need_user_confirm`), rebuilds router for new plan, emits `plan_replaced`, and persists `plan_epoch`/`plan_hash_history`/`plan_snapshot` in checkpoint.
  - `AISRS-RUN-020` (plan diff text/json path wired to engine diff)
  - `AISRS-RUN-021` (replay trace/checkpoint path with until-node, text/json output)
  - `AISRS-RUN-022` (run workflow 0.0.3 mode: workspace+workflow validation, compile_workflow, dry-run or execute via engine)
  - `AISRS-AGENT-001` (interactive `ais-runner agent` outer loop: run→pause→human/yolo decision→send engine commands→continue)
  - `AISRS-AGENT-002` (LLM provider abstraction + tool-calling adapter): `LlmBrain<P: ais_llm::LlmProvider>` maps typed tool calls (`confirm`, `cancel`, `send_engine_command`) into engine commands.
  - `AISSLIM-RS-001` (unified decision entry): runner `agent` path now uses a single `DecisionPolicy` interface with one concrete state machine (`AgentDecisionPolicy`) to handle `safe|assist|yolo`, optional assist LLM auto-approval, and manual fallback.
  - `AISSLIM-RS-003` (candidate context budget): when `agent` is given `--workspace`, runner builds executable index candidates from workspace protocols/packs, injects a capped index-only candidates payload into LLM context (`--max-index-candidates`, default `24`), and exposes detail lookup by `ref` via tool-calling (`get_candidate_detail`) instead of inlining detail cards.
  - `AISSLIM-RS-004` (reason_code stabilization): runner now consumes stable `reason_code` fields from engine events for pause/error summaries, and replace-plan rejection/confirmation paths emit stable reason_code enums instead of relying on free-text reason matching.
  - `AISSLIM-RS-002` (demo 通道隔离): `agent` 增加 `--profile standard|demo-scripted`；`--llm-script-jsonl` 归入 `Demo Options`，并由 profile 约束启用，默认 `standard` 路径不依赖脚本化 LLM 输入。
  - `AISINT-RS-001` (intent CLI scaffold): `agent` 输入改为三选一（`--plan|--intent|--intent-file`）并在 CLI 层强约束。
  - `AISINT-RS-002/003/004` were replaced by segmented planning flow; one-shot `propose_plan/revise_plan` path is removed from runner default execution.
  - `AISNEXT-RS-003` (segmented planning/execution loop): intent mode now supports `plan.begin` + `plan.propose_segment/revise_segment` with segment-by-segment `compile_plan_sketch -> replace_plan -> execute` closure, checkpointed handoff, and `state_summary` feedback between rounds.
  - segment revise merge now replaces prior nodes of the same `extensions.plan_sketch.segment_id` instead of append-only merge, preventing duplicate node ids across repeated repair rounds.
  - segmented plan merge metadata keeps `segment_count` under `plan.meta.extensions.segment_count` (schema-safe), avoiding invalid `replace_plan` payloads caused by unknown `meta` keys.
  - segmented planner tool-call decoding now accepts provider quirks where `plan.propose_segment.segment` is returned as a JSON string and where `cursor_next` is numeric (coerced to string), reducing unnecessary planner round failures.
  - for proposed segment drafts, `cursor_next` is optional: if omitted by model output, runner falls back to `segment.cursor_out` for continuation.
  - `AISNEXT-SLIM-DEL-001` (hard delete one-shot planner): removed legacy one-shot `propose_plan/revise_plan` runtime path and switched `agent --intent|--intent-file` to segmented-only execution with workspace candidates.
  - `AISNEXT-RS-004` (checkpoint idempotency contract): runner persists tx/approval ledgers into checkpoint and reconciles persisted side-effects on resume to prevent duplicate sends after crash/restart windows.
  - resume reconciliation semantics: `sent` side-effects are not treated as completed; runner delegates reconciliation to chain executors, and unresolved `sent` records pause execution with `side_effect_reconcile_pending:*` instead of replaying value-moving calls.
  - `AISNEXT-TEST-002` (crash-injection idempotency): crash-window tests cover checkpoint side-effect replay protection and explicitly verify that removed runtime-scan fallback no longer guards replay when checkpoint side-effects are absent.
  - `AISNEXT-ARCH-006` (side-effect + 幂等恢复矩阵): runner tests覆盖 `sent` side-effect 防重放、`reverted` side-effect 不误完成，以及“checkpoint side-effect 不可绕过 execution.type 未注册校验”。
  - `AISNEXT-ARCH-002` (SideEffect contract alignment): runner side-effect model aligns to `ais-engine` `CheckpointSideEffectRecord` contract and consumes engine-emitted side-effect events.
  - `AISNEXT-ARCH-004` (event-driven checkpoint ledger): runner checkpoint ledger is now event-driven only (`side_effect_observed`), and runtime `nodes.*.outputs` scanning fallback is removed.
  - `AISNEXT-ARCH-005` (compat hard-delete): checkpoint ledger now keys strictly by `idempotency_key` (no `tx_hash/no_tx_hash` fallback), and runner config no longer hard-rejects non-`eip155`/`solana` chain IDs so external chains can be served by registered plugin routes; side-effects are now produced at executor boundary and propagated by engine events.
  - `AISNEXT-ARCH-003` (execution type capability registry): runner route registration now consumes `ais-engine` route presets for built-ins (`EvmCore/EvmPlugin/SolanaCore`) and runtime plugin execution-type registration for offchain handlers (no `OffchainApyPlugin` hardcoded preset).
  - `AISNEXT-RS-005` (safety governance chain wiring): runner now consumes engine-side safety hook/sanitize/hard-block behavior and sanitizes candidate tool outputs (`list_candidates` / `get_candidate_detail`) before passing them into LLM tool loops.
  - `AISNEXT-RS-006` (token budget + compact cards + event summary budget): runner now applies JSON budget compaction on candidate/tool payloads and planner feedback summaries, and caps detail lookups (`get_candidate_detail`) to a fixed window with truncation metadata.
  - `AISNEXT-TEST-004` (segmented e2e fixture): added local offchain segmented-intent fixture (`fixtures/runner-local/intent-segmented-offchain-transfer`) and an end-to-end test that runs `plan.begin -> propose_segment x2`, executes balance-query segment, checkpoints handoff state, then pauses second transfer segment on `need_user_confirm`.
  - `AISRS-CTRL-010` (runtime-controls segmented e2e): added scripted segmented fixture flow that first emits invalid `until` (compile failure), then repairs via `plan.revise_segment` with valid `until/retry/timeout_ms`, and validates engine `until_retry` pause plus next-run completion (`fixtures/runner-local/intent-segmented-offchain-transfer/llm/segmented.until-retry.repair.template.jsonl`, test: `segmented_intent_fixture_revise_with_until_retry_then_complete`).
  - `AISRS-MIN-008` (format-failure fixture coverage): added scripted repair flow for malformed string `segment` output plus cross-segment `depends_on` compile failure, and validates revise-path recovery with compiled `assert/branch` control steps (`fixtures/runner-local/intent-segmented-offchain-transfer/llm/segmented.format-repair.template.jsonl`, test: `segmented_intent_fixture_repairs_format_then_compiles_assert_branch_segment`).
  - `AISNEXT-TEST-005` (large catalog stress): added large-catalog budget tests for segmented intent candidate payload compaction (`list_candidates` / `get_candidate_detail`), ensuring bounded payload size and stable truncation markers.
  - candidate discovery now supports `catalog.search` (keyword/risk/chain/limit filters) on top of snapshot/detail tools; search operates on full executable candidates while `list_candidates` remains index-windowed for token stability.
  - `AISINT-RS-005` (confirm UX convergence): `need_user_confirm` prompt now prints chain/action/risk and extracted amount/asset/target highlights from confirmation summary; manual prompt adds `always_approve_this_run|aa` for per-run sticky auto-approve.
  - segmented planner prompt enforces candidate-first discovery (`list_candidates`/`catalog.search`) and `plan.begin -> plan.propose_segment|plan.revise_segment` tool ordering.
- segmented planner prompt contract is detect-free (`ValueRef` = `lit/ref/cel/object/array`) and explicitly documents `asset` input shape (prefer object with `address/chain_id`).
- prompt/contracts guidance for `asset` now prefers `object.address + object.chain_ref`; compiler normalizes `chain_ref -> chain_id` for execution bindings.
  - segmented planner prompt now enforces write-safety via deterministic CEL + explicit query/assert/branch guards (no hardcoded template names in guide topics).
  - segmented planner system prompt now uses a modular context builder (`base rules` / `phase rules` / `contracts summary` / `pack summary` / `workspace summary`) with stable `Prompt-Version` and content hash for easier regression diffing.
  - `AISRS-AGENT-007` (schema/tool contract lookup): segmented planner adds `guide.get` tool (propose/revise phase) to fetch embedded JSON schemas and guide topics (`cel|valueref`) with bounded payload budgeting/caching, and prompts now explicitly direct model to query schema/tool guides before finalize when uncertain.
  - segmented planner anti-misuse hardening: prompts now explicitly forbid using `catalog.search` for control semantics (`assert/branch/until/retry`) and require `guide.get`; runtime tool layer returns structured `hint` (`next_tool=guide.get`) when such control-term catalog searches are attempted.
  - lightweight `PlanningMemory` caches `list_candidates` / `catalog.search` / `get_candidate_detail` / `guide.get` tool results by `(snapshot_hash,tool,args_hash)` so repeated calls reuse prior tool outputs and reduce token churn even if planner session id changes.
  - segmented agent persists bounded `PlanningMemory` into checkpoint `extensions.planning_memory` and restores it on resume (`snapshot_hash` scoped, with entry/size budgets), reducing repeated discovery tool calls after interruption.
  - segmented loop now auto-repairs malformed planner tool outputs (for example invalid `plan.propose_segment` payload shape/JSON) by feeding a bounded `previous_error` back into `plan.revise_segment` instead of hard-failing on first decode error.
  - planner-format repair payloads include `sub_reason_code` + `hint` + `expected_finalize_tool` to bias the model toward fixing output shape without rewriting plan semantics.
  - planner/compile previous_error payloads now include `phase_reason_code` and explicit `repair_order=[shape,ref,slot,semantic]`; grounding/todo bootstrap failures are tagged with phase-scoped reason codes (`grounding.*` / `todo.*`) for clearer diagnostics.
  - `AISRS-FT-001` (input_store model): added `agent/facts.rs` with `FactStore` (`seed/observed/derived` layers), source-priority merge/upsert, and planning-safe payload export (`facts` + `meta`).
  - `AISRS-FT-002` (owner/default wallet seeding): segmented agent now builds initial facts from runtime fallbacks and signer-derived EVM addresses, then injects them into planner state summary before first `plan.propose_segment`.
  - `AISRS-FT-003` (missing_required_input protocolization): `SegmentDraft::Unavailable` now supports `questions[]` extracted from `error.details.questions[]`, prompt contracts require this shape for missing facts, and runner pauses on `missing_required_input` with payload persisted under runtime agent context.
  - `AISRS-FT-004` (missing-input user interaction): in TTY mode runner asks user to resolve planner `questions[]` (option-select + custom value), writes answers into `runtime.inputs` and `input_store` (`user_input` provenance), then continues with `plan.revise_segment` instead of terminating.
  - `AISRS-FT-005` (todo model + 状态机): runner introduces `TodoBoard` (`id/title/required_facts/produced_facts/acceptance/status/blocked_reason`) with explicit transitions `todo -> in_progress -> done|blocked`.
  - `AISRS-FT-006` (todo-first segmented loop): segmented intent execution now records/updates `runtime.agent.todo_progress`, scopes planner context with `state_summary.todo_state`, and host-enforces v1 `1 todo = 1 segment`.
  - `AISRS-FT-007` (write satisfiability gate templates): transfer/swap-like action segments are preflight-validated for gate chain (`query -> assert|branch -> action`), required query presence, and token decimals availability before compile/execute.
  - `AISRS-FT-008` (fact/todo checkpoint persistence): segmented mode checkpoint extensions now roundtrip `input_store` + `todo_progress`, and restore-side merges `input_store` into planner `FactStore` projection on resume.
  - `AISRS-FT-009` (`plan.propose_todos`): segmented planner新增 `propose_todos` phase 与 `plan.propose_todos` finalize tool；runner 在无历史 todo_progress 时先规划 deterministic todos，再由 host 规范化并落地到 `runtime.agent.todo_progress`（失败时降级 bootstrap todo，不中断主流程）。
  - `AISRS-FT-010` (`todo_id` segment/receipt binding): runner 在执行前将 `todo_id` 绑定到 `segment.extensions.todo_id`，执行事件追加 `event.extensions.agent.{todo_id,segment_id,step_id}`，并将 round 级执行回执回写到 `todo_progress.todos[].receipt`（status/paused_reason/node_ids/completed_node_ids/tx_hashes/event_types/event_count）。
  - `AISRS-FT-011` (fact staleness + refresh): `FactStore` 增加 `stability/observed_at_ms` 元数据并区分 stable/volatile facts；写前门控对 volatile facts（balance/allowance）执行 freshness 检查，若未在同段 refresh query 且缓存过期则返回 `stale_volatile_fact`，强制先查后写。
  - `AISRS-FT-013` (orchestrator 模块化): segmented 主循环已拆分到 `agent/orchestrator.rs`，按 `plan_round -> compile_guard -> execute_round -> checkpoint_round` 阶段执行；`mod.rs` 仅保留入口装配。
  - `AISRS-FT-014` (context projection + diff): 新增 `agent/context_view.rs` 的 `PlanningContextManager`，统一 `state_summary` 投影并通过稳定哈希标注 `context_unchanged`，同时对 fact payload 做有界压缩。
  - `AISRS-FT-015` (checkpoint extensions typed codec): 新增 `agent/checkpoint_ext.rs` 统一 `planning_memory/input_store/todo_progress/intent_facts` 编解码；segmented checkpoint 保存改为 typed `encode_updated`，并保留 unknown extensions 透传（`input_store` key 不再写入/恢复）。
  - `AISRS-FT-016` (`stores` fact backfill): segmented 执行回调按 `step.stores` 从 `runtime.nodes.<node_id>.outputs` 映射并回填 `input_store`（`Observed + QueryObserved + provenance`），回填结果随 checkpoint 持久化供后续轮次直接消费。

- `AGT-P3-000` (`single-source freeze`): 迁移开关与模式契约冻结完成（`AIS_RUNNER_INPUT_STORE_MIGRATION_MODE`）。
- `AGT-P3-010` (`input_store` core): 新增 `agent/input_store.rs`，提供 typed 真源缓存（`upsert/get/has/list_refs/to_runtime_projection`）与 key 归一化约束。
- `AGT-P3-020` (`input write path`): grounding/missing_input/startup seed 写路径统一经 InputStore 入口，runtime 仅保留兼容投影镜像。
- `SS-P1-030` (`grounding/missing_input canonical input writes`): grounding resolve 与 missing-input 用户回答在输入语义层仅写 `inputs.*` canonical key（移除 `<slot>` + `inputs.<slot>` 双写），并清理 `input.*` legacy 输入 alias 分支。
- `AGT-P4-080` (`todo receipt tx_hashes aggregation fix`): `todo_progress.todos[].receipt.tx_hashes` 改为按当前 todo segment 的 `side_effect_observed.record.tx_hash` 聚合（不再扫描 runtime outputs），确保与 checkpoint `side_effects` 账本一致；checkpoint extension roundtrip 回归覆盖 receipt `tx_hashes` 保留。
- `AGT-LI-002` (`side-effect single-source convergence`): todo receipt `tx_hashes` 与 `runtime.nodes.<node_id>.outputs.tx_hash` 均改为从 checkpoint `side_effects` 账本投影（ledger single source）；执行后回执和节点输出会被同一账本聚合结果对齐，避免节点输出与 ledger/receipt `tx_hash` 漂移。
- `AGT-LI-005` (`todo receipt tx_hashes restore hardening`): segmented restore 在 `TodoBoard` 恢复前会基于 checkpoint `side_effects` 账本重算 `runtime.agent.todo_progress` 中各 todo receipt `tx_hashes`（按 receipt `node_ids`），并同步修正 `runtime.nodes.<node_id>.outputs.tx_hash`；checkpoint extension decode 兼容 legacy receipt `tx_hashes` 形态（string/null -> array）且 roundtrip 覆盖多 tx-hash 持久化。
- `AGT-P4-112` (`query backfill contract unification`): `segment_exec` 的 query `stores` 对输入语义键统一写入 `inputs.<canonical_leaf>`，并由 `FactStore -> InputStore -> runtime.inputs` 与 checkpoint extensions 使用一致语义恢复。
- `AGT-P3-040` (`input read path`): 缺参判定/known refs/pause backflow 优先读取输入语义真源。
- `AGT-P3-050` (`context projection`): `input_slots/input_registry/canonical_context` 改为 InputStore 优先投影，runtime.inputs 仅兜底。
- `AGT-P3-060` (`runtime.inputs projection adapter`): 提供 `InputStore -> runtime.inputs` 单向投影能力，并记录 drift/repair 计数（适配器能力保留）。
- `SS-P2-020` (`remove FactStore type from segmented main-flow signatures`): orchestrator compile/autofill/todo-acceptance 输入判定不再通过主流程函数签名传递 `FactStore`；compile known refs 改为来自 `state_summary.input_registry/input_slots/intent_slots`，missing-input 已有值判定与 todo acceptance 改为 `InputStore/IntentContext` 读取路径；`compile_segment_plan_with_snapshot_hash_and_facts` 仅保留 `#[cfg(test)]` 用于现有测试编译入口。
- `SS-P1-020` (`remove runtime_inputs projection adapter business call`): segmented orchestrator 不再在执行前/输出前主动调用 `project_runtime_inputs`；运行日志不再强调 `runtime_inputs_projection` 阶段事件，适配器收口为非业务主流程调用。
- `SS-cleanup` (`warning cleanup`): 仅测试使用的 projection helper 已收敛到 `#[cfg(test)]`（`project_runtime_inputs` 及其辅助结构、`InputStore::to_runtime_projection/len/is_empty`），生产构建不再携带对应 dead-code 路径。
- `AGT-P3-070` (`checkpoint/restore`): checkpoint extensions 使用 typed `input_store` 载荷，restore 仅走 `input_store` 并重建 `FactStore` 输入投影。
- `AGT-P3-080` (`legacy cleanup`): 删除 runtime 输入直读回退和 `known_input_refs` legacy helper，收敛到单一语义入口。
- `AGT-P3-090` (`single-source tests`): 覆盖 `token.decimals` 回填、`missing_required_input` 解除、restore/projection 一致性回归。
- `AGT-P3-100` (`docs finalization`): README + design + final report 口径对齐，统一边界/流程/迁移/观测/排障与命令规范。

## InputStore 入口与边界

- 目标：集中管理输入槽位（owner/token/chain/amount 等）的写入顺序、元信息与投影结果，输出统一的 `runtime.inputs` 视图给执行层。
- 边界（P3 收敛后）：InputStore 已覆盖输入全链路关键路径（写入、读取、context 投影、runtime 投影适配、checkpoint restore）；`runtime.inputs` 保留执行侧兼容定位。
- 公开入口：
  - `InputStore::upsert`
  - `InputStore::get`
  - `InputStore::has`
  - `InputStore::list_refs`
- `InputStore::to_runtime_projection` 仅用于测试辅助（`#[cfg(test)]`），不属于生产主流程 API。
- 迁移模式：`legacy` / `shadow_writes` / `read_through` / `single_source` / `enforced_single_source`，默认 `single_source`，通过 `AIS_RUNNER_INPUT_STORE_MIGRATION_MODE` 控制。
- 流程：`input write -> InputStore canonical refs -> context/read path -> runtime.inputs compatibility/checkpoint extension`（projection adapter 不再作为 orchestrator 主流程固定阶段）。
- 观测项：`runtime.agent.input_projection.{sync_total,drift_total,repair_total,legacy_input_path_hits,legacy_input_path_hits_total,strict_guard_triggered}`。
- strict guard：`enforced_single_source` 模式下，如本轮投影命中 legacy 输入路径（`legacy_input_path_hits > 0`），会触发明确失败（panic）阻止 mixed-source 继续执行。
- 排障顺序：先确认 migration mode，再检查 `runtime.agent.missing_required_input` 与 input registry，再定位 projection drift 与 fallback 计数。
- 依赖：`serde_json`（值序列化）、`serde`（可序列化元数据）和 `agent/input_normalize.rs`（canonical key 解析）。

  - `AISRS-FT-017` (write-gate policy modularization): 写前门控已拆分到 `agent/write_gates.rs`，并移除基于 `candidate_ref/id` 的 `transfer|swap` 字符串启发式；改为结构化字段判定（`risk_tags/requires_queries/params`）+ `write_gate` 显式覆盖配置。
  - `AISRS-FT-018` (error state convergence): 新增 `agent/error_state.rs` 统一 planning/compile/execution 错误分类与 payload；planner 输出修复改为模式表驱动，compile 错误统一封装为 `phase=compile`，并收敛 `previous_error` 与 `state_summary` 更新路径。
  - `AGT-P2-230` (reason/subreason enumization): `agent/error_state.rs` now emits `reason_code` / `sub_reason_code` through serde-mapped enums (planning/execution/compile), and compile/grounding/todo base reason decode uses typed known+raw passthrough mapping so existing wire strings remain unchanged.
  - `AISINT-TEST-001` (CLI intent 参数层): covers `--intent-file` parsing and `--intent` vs `--intent-file` mutual exclusion.
  - `AISINT-TEST-002` (planner 回路异常): covers empty tool-calls, invalid plan payload, and planner tool-round limit.
  - `AISINT-TEST-003` (资金安全核心路径): covers deny keeps run paused/uncompleted and assist threshold overflow falls back to manual confirmation.
  - standard profile now supports real LLM initialization from runner config `llm` section (`provider/model/api_key/api_base`) via `ais-llm::providers::build_provider`.
  - runner `llm` config now supports provider chain assembly (`retry/fallback/rotation`) via `ais-llm::providers::build_provider_chain`, with unified provider error classification.
  - `AISSLIM-TEST-001` (资金安全核心路径矩阵): runner execute tests now cover unregistered execution-type rejection on execute path and checkpoint resume decision consistency for `need_user_confirm` (`paused_reason` + `confirmation_hash` stability).
  - `AISRS-POL-001` (for `agent --pack`: maps pack approvals + chain scope + plugin execution allowlist into engine policy enforcement options, and validates approvals threshold configuration)
  - `AISRS-POL-001` assist-mode extension: when `approvals.mode=assist` and pack defines `llm_may_approve_max_risk_level`, runner can auto-approve low-risk `need_user_confirm` via LLM tool-calls (demo scripted: `--profile demo-scripted --llm-script-jsonl <file>`; standard: runner config `llm` provider), with manual fallback.
  - `AISRS-POL-002` (agent pause/confirm behavior is schema/constraints-driven via `node.extensions.policy.*` + risk-threshold policy; no swap/approve method-name heuristics)
  - `AISRS-PLUG-001` (router assembly registers core/plugin execution handlers separately; plan preflight fails fast when `execution.type` is unregistered for node chain)
  - `AISRS-PLUG-002` (offchain plugin sample: runner config can register `offchain_apy_query` per chain with `allowed_domains` + timeout/retry policy)
  - workflow execute mode can evaluate top-level `workflow.outputs` against final runtime and write them to a dedicated JSON file via `--outputs`.
  - runner `rpc_url` validation accepts `http(s)` and `ws(s)` for chain endpoints.
  - chain `timeout_ms` now maps to EVM RPC client request timeout middleware.
  - Workspace loader tests keep schema-valid protocol fixtures to match strict parser+schema validation behavior
  - `--verbose` runtime event printing for `run plan` / `run workflow` (stderr event lines for easier trace/debug); `error` events additionally print full event `data` JSON for assert/condition/executor diagnostics.
  - Minor cleanup: simplified parser error mappers and state init branches to reduce boilerplate and clippy noise.
- Runner delegates EVM read/call/rpc transport to `ais-evm-executor` Alloy-backed sender adapters (no local duplicate EVM transport implementation in `ais-runner`)
- Planned next:
  - wire pack policy into `agent` and align `need_user_confirm` UX with confirmation hash/summary
  - add production provider adapters (OpenAI/Anthropic/etc.) over `ais-llm::LlmProvider`
  - wire real Solana RPC client factory into `config` executor assembly path
  - runner integration polish and fixtures coverage

## P3 Regression Command Baseline

- Working directory: `/home/xcshuan/work/owlia/ais`
- Rule: one `cargo test` command uses one filter only.

```bash
cargo test --manifest-path rust/ais-rs/Cargo.toml -p ais-runner agent::tests:: -- --nocapture
cargo test --manifest-path rust/ais-rs/Cargo.toml -p ais-runner agent::orchestrator::tests:: -- --nocapture
cargo test --manifest-path rust/ais-rs/Cargo.toml -p ais-runner intent_segmented -- --nocapture
bash -n rust/ais-rs/scripts/agent_regression_baseline.sh
bash rust/ais-rs/scripts/agent_regression_baseline.sh --group all
bash rust/ais-rs/scripts/agent_regression_baseline.sh --group wave2
bash rust/ais-rs/scripts/agent_regression_baseline.sh --group wave3
bash rust/ais-rs/scripts/agent_regression_baseline.sh --group wave4
```

## AGT-P4-030 Wave-P4-B Live Smoke (UTC 2026-02-28)

- Result: blocked at provider call stage (`ground_intent`), config parse passed and agent flow started.
- Repro commands (repo root: `/home/xcshuan/work/owlia/ais`):

```bash
cargo run --manifest-path rust/ais-rs/Cargo.toml -p ais-runner -- agent \
  --intent-file rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --profile standard \
  --approvals-mode safe \
  --events-jsonl /tmp/agt-p4-030-runner-local.events.jsonl \
  --trace /tmp/agt-p4-030-runner-local.trace.jsonl \
  --checkpoint /tmp/agt-p4-030-runner-local.checkpoint.json \
  --verbose \
  --format text
```

- Key output:
  - `[agent.phase_machine] main_flow_enter phase=ground_intent transitions=2`
  - `llm provider call failed: request failed endpoint=https://openrouter.ai/api/v1/chat/completions: error sending request`
- Notes:
  - `runner.local.yaml` uses inline `api_key` and `rpc_url`, so it does **not** require `OPENROUTER_API_KEY` / `EVM_RPC_URL`.
  - env vars are required only when using placeholder configs under `rust/ais-rs/fixtures/runner-local/llm-providers/config/*.yaml`.

### Ops diagnostics sequence (logs/checkpoint/events-jsonl/missing-input)

```bash
# 1) If using placeholder config (`llm-providers/config/*.yaml`), check envs first
env | rg '^(OPENROUTER_API_KEY|GROQ_API_KEY|ANTHROPIC_API_KEY|EVM_RPC_URL)=' || true

# 2) Run with persisted artifacts
cargo run --manifest-path rust/ais-rs/Cargo.toml -p ais-runner -- agent ... \
  --events-jsonl /tmp/agent.events.jsonl \
  --trace /tmp/agent.trace.jsonl \
  --checkpoint /tmp/agent.checkpoint.json \
  --verbose --verbose-llm --format text

# 3) If paused on missing input
rg -n "missing_required_input|need_user_input" /tmp/agent.events.jsonl
rg -n "runtime.agent.missing_required_input|known_refs|missing_refs" /tmp/agent.checkpoint.json
```

- Expected artifact behavior:
  - if config parse fails early, `/tmp/agent.events.jsonl|trace|checkpoint` may not be created;
  - if provider call fails after phase start (current run), artifacts may exist but flow fails in `ground_intent`;
  - when run reaches orchestrator, use above grep paths to diagnose missing-input payload and recovery context.

### Minimal executable fallback (no live provider)

```bash
cargo run --manifest-path rust/ais-rs/Cargo.toml -p ais-runner -- agent \
  --intent-file rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.success.jsonl \
  --approvals-mode safe \
  --events-jsonl /tmp/agt-p4-030-demo.events.jsonl \
  --trace /tmp/agt-p4-030-demo.trace.jsonl \
  --checkpoint /tmp/agt-p4-030-demo.checkpoint.json \
  --verbose --format text
```
