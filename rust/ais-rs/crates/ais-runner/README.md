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
- `list_candidates` returns protocol-grouped discovery snapshots (`protocol/chains/actions[]/queries[]`) plus `execution_plugins`; each action/query card is compact (`ref/chains/required_inputs`) to minimize token overhead; `catalog.search` returns ref-first compact matches (`ref/kind/chains?/risk_level?`) for targeted filtering; call `get_candidate_detail` for full params/returns/risk/execution details
- `capability_view` is semantic-ready (`ais-agent-capability-view/0.0.2`): per protocol it exposes `topics[]` + `topic_cards[]` in addition to raw `actions/queries/required_inputs`; topic values are declaration-driven (`extensions.agent.topic(s)` / `risk_tags` / `meta.tags`) and no longer inferred from action/query name substrings
- planner rounds enforce strict tool-call FSM: begin round only `plan.begin`; propose/revise rounds only discovery tools + matching finalize tool, with finalize at most once and always last
- planner rounds now include `ground_intent` phase: extract initial `resolved_inputs/intent_facts` with confidence before todo planning; low-confidence fields are returned as questions
- grounding decode hardening: if model returns `status=proposed` but omits `ready_for_todos`, runner infers readiness only when no questions are present and resolved inputs are non-empty (prevents false `missing_required_input` pauses on well-grounded payloads)
- finalize tool input schemas are sourced from embedded `ais-agent-planning-tools/0.1.0` definitions (`segment_payload`), avoiding runner-local schema drift
- finalize tool-call contracts are conditional:
  - `plan.propose_segment` / `plan.revise_segment` with `status=proposed` MUST include a `segment`
  - with `status=invalid|unavailable` MUST include an `error.reason_code`
- segmented step kinds support `query|action|assert|branch`
- `plan.begin.cursor` accepts string/number and is normalized to string for session state (`"0"` style cursor is valid)
- planner tool-call decoding is strict: `segment` must be a JSON object (stringified JSON payloads are rejected with deterministic repair reason codes)
- `get_candidate_detail` keeps minimal callable signatures stable for LLM planning (`params[].name/type/required`, `returns[].name/type`) while still applying payload budget compaction
  - propose/revise prompts include explicit contracts for step fields, ValueRef (`lit/ref/cel/object/array`), CEL namespaces, and dependency references
  - propose/revise prompt contracts explicitly forbid legacy branch-tree step fields (`if_true/if_false/then/else/children`) and require branch paths to be encoded as flat steps with `when.cel + depends_on`
- segmented planner `state_summary` now carries a structured `fact_store` payload (`facts` + `meta` with source/provenance), so model-side planning can consume runtime/config-derived facts deterministically
- segmented planner `state_summary` also carries `input_slots` (`resolved`/`missing`/`canonical_refs`) so model output can anchor to stable `inputs.*` refs instead of guessing input paths
- segmented planner `state_summary` now carries `input_registry` (`known_refs` + entry metadata) and planner prompts require `inputs.*` refs to come from this registry
- segmented planner `state_summary` now carries chain-agnostic `canonical_context` (`chain_refs/account_refs/asset_refs/amount_refs`) so planning can reason across EVM/Solana-style account/asset shapes without EVM-only field assumptions
- segmented planner `state_summary` now carries `tool_memory_projection` (bounded recent memory for `catalog.search` / `get_candidate_detail` / `guide.get`) so planner can reuse high-value discovery/schema context before issuing duplicate tool calls
  - projection applies stronger dedupe (cross-entry `ref` dedupe and repeated schema/topic collapse) and guide priority ranking (`ais-plan-sketch` / `cel` first) to keep high-signal context within token budget
  - guide memory is structured as keyed maps (`recent.guide.schema.<schema_id>` / `recent.guide.topic.<topic_id>`), and schema `full` payloads replace prior digest entries for the same schema id
  - projection token budget is adaptive (`800~1500`): derived from current context headroom (`context_remaining_tokens/context_soft_limit_tokens`) when available, otherwise by absolute remaining-token fallback
- segmented runner also maintains `runtime.agent.todo_progress` and injects `state_summary.todo_state` (`current_todo` + progress counters), so each planning round is scoped to one explicit todo objective
- initial `fact_store` seeds flattened runtime `inputs` into both `<slot>` and canonical `inputs.<slot>` keys (for example `owner` + `inputs.owner`, `token.address` + `inputs.token.address`) in addition to runtime fallback owner/wallet and signer-derived addresses (`owner_by_chain.<chain>`), with priority order `user > query > config > runtime > derived > intent`
- `guide.get` request shape is strict and string-only: `{schema:"ais-plan-sketch/0.1.0"}` or `{topic:"cel"}`; object/nested compatibility shapes are rejected
  - schema responses default to compact `digest` mode (`schema.digest`) instead of returning the full schema JSON; use `{schema:"...",full:true}` only when full schema payload is strictly required
  - cache semantics are canonical by schema/topic id: when `{full:true}` is requested after a cached digest, runner refreshes and replaces the cached entry with full schema payload instead of keeping duplicate digest/full copies
- `plan.check_segment` runs compile-only segment validation and returns structured issues (`ok=false` + `issues[]`) without mutating active plan or executing nodes
- runner enforces a successful `plan.check_segment` (`ok=true`) before accepting `plan.propose_segment`/`plan.revise_segment` outputs with `status=proposed` (unavailable/invalid drafts are exempt from this gate)
- compile guard validates `inputs.*` ValueRef references against known input registry and returns structured `unknown_input_ref` issues with suggested canonical refs
- segmented draft step contract is strict: each `segment.steps[]` must include `id/kind/candidate_ref/inputs`
  - when planner omits `candidate_ref`, runner emits targeted diagnostics with missing step ids/kinds and classifies retry reason as `missing_candidate_ref` for deterministic repair prompts
  - planner-output repair payload now includes `previous_error.last_failed_finalize` (failed finalize tool call args + assistant snippet, compacted), so revise rounds can patch the previous draft instead of regenerating from scratch
- `segment.steps[].depends_on` may only reference step ids in the same segment (cross-segment ids like `seg_1/...` are invalid)
- planner missing-input contract: when required facts are missing, planner should return `status=unavailable` + `error.reason_code=missing_required_input` + `error.details.questions[]`
- runner treats `missing_required_input` as a pause (not hard failure) and records machine-readable payload at `runtime.agent.missing_required_input`
- execution-stage `need_user_input` pauses with `reason_code=missing_required_input` are now normalized into the same payload contract (`missing_refs/suggested_paths/questions`), so缺参不会落入 `need_user_confirm`.
- on interactive TTY runs, runner prompts user to answer `questions[]` (choose option index or enter custom JSON/string), backfills `runtime.inputs.*` + planning `fact_store`, and immediately retries planning via `plan.revise_segment`
- segmented orchestrator also handles execution-time missing-input pauses by prompting/回填/自动继续; unresolved answers keep pause as `missing_required_input` and block current todo deterministically.
- todo-first loop is host-enforced: each round advances exactly one current todo (`todo -> in_progress -> done|blocked`), and non-final rounds auto-open follow-up todo entries
- host-side write gate validation blocks transfer/swap segments without `query -> assert|branch -> action` dependency chain and enforces token-decimals availability for asset writes
- compile segment (`plan-sketch` IR) to executable `ais-plan`
- append via guarded `replace_plan`
- execute + checkpoint, then continue next segment

### Demo-scripted profile (deterministic)

Use fixture `rust/ais-rs/fixtures/runner-local/intent-native-erc20-transfer`:

```bash
cargo run -p ais-runner -- agent \
  --intent-file fixtures/runner-local/intent-native-erc20-transfer/intent/intent.txt \
  --workspace fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config fixtures/runner-local/intent-native-erc20-transfer/config/runner.local.yaml \
  --runtime fixtures/runner-local/intent-native-erc20-transfer/runtime/runtime.local.json \
  --profile demo-scripted \
  --llm-script-jsonl fixtures/runner-local/intent-native-erc20-transfer/llm/intent-native-erc20.success.jsonl \
  --approvals-mode safe \
  --format text
```

### Standard profile (real provider)

Use one template under `rust/ais-rs/fixtures/runner-local/llm-providers/config/` and set env keys:

```bash
cargo run -p ais-runner -- agent \
  --intent "check balances then transfer if both >100" \
  --workspace fixtures/runner-local/intent-native-erc20-transfer/workspace \
  --pack fixtures/runner-local/intent-native-erc20-transfer/workspace/safe-defi.ais-pack.yaml \
  --config fixtures/runner-local/llm-providers/config/openrouter.config.yaml \
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
  - `context_remaining_tokens` now means per-call context-window headroom (soft limit - current request input tokens).
  - when context usage exceeds 90% (remaining ratio `<=10%`), a dedicated pressure strategy is applied: trim duplicate projections first (for example `input_slots.canonical_refs`), drop low-priority heavy sections (`capability_view.protocols`), and aggressively compact large blobs (`tool_memory_projection`, `previous_error.last_failed_finalize`).

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
- Checkpoint persistence now includes `approvals_ledger` + `side_effects` (tx hash/idempotency key); on resume, runner marks only `confirmed` side-effects as completed and asks chain executors to reconcile `sent` side-effects before continuing (pending reconcile pauses run to avoid blind replay; reverted reconcile statuses pause with `side_effect_reconcile_reverted:*`).
- runner records normalized side-effect lifecycle summary at `runtime.agent.side_effect_lifecycle` (`sent/confirmed/reverted` counters + per execution type breakdown), and `state_summary` projects this structure for segmented planning context.
- segmented checkpoint extensions also persist `fact_store` + `todo_progress` (with `planning_memory`) so resumed runs keep planning context and previously supplied facts.
- segmented checkpoint extensions also persist typed `intent_facts`; restore path injects them back into `runtime.agent.intent_grounding.intent_facts` and merges into planning `fact_store`.
- `fact_store` overwrite guard keeps intent semantic facts stable against volatile query observations (for example balance/allowance refresh values do not rewrite intent constants).
- segmented planner `state_summary` now applies staged context-budget projection (`balanced/tight/minimal`) with stable clipping order; key slots (`owner/wallet/token/amount/chain`) are preserved first and `context_budget` metadata is exposed to the model.
- agent final output (`text`/`json`) includes session-level `llm_usage` totals for segmented planning runs.

## Dependencies

- `ais-sdk`: parse + dry-run planner APIs
- `ais-llm`: provider-agnostic LLM tool-calling types + provider registry/factory integration (`LlmBrain` + `build_provider`)
- `ais-offchain-executor`: offchain `offchain_apy_query` plugin handler registration
- `clap`: CLI parsing
- `serde_json`, `serde_yaml`: runtime file decoding
- `thiserror`: CLI/domain errors

## Current status

- Implemented:
  - `AISRS-RUN-001` (CLI 命令骨架 + `--help` smoke test)
  - `AISRS-RUN-002` (workspace 目录加载与分类：protocol/pack/workflow/plan，含 issues 输出)
  - `AISRS-RUN-003` (runner config 解析/校验 + EVM/Solana executor 装配 + plan chain 缺失校验)
  - `AISRS-RUN-010` (run plan dry-run text/json, includes `main.rs` CLI dispatch and `run_test.rs`)
  - `AISRS-RUN-011` (run plan execute loop + events-jsonl sink + trace sink + checkpoint save/restore)
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
  - `AISRS-FT-001` (fact_store model): added `agent/facts.rs` with `FactStore` (`seed/observed/derived` layers), source-priority merge/upsert, and planning-safe payload export (`facts` + `meta`).
  - `AISRS-FT-002` (owner/default wallet seeding): segmented agent now builds initial facts from runtime fallbacks and signer-derived EVM addresses, then injects them into planner state summary before first `plan.propose_segment`.
  - `AISRS-FT-003` (missing_required_input protocolization): `SegmentDraft::Unavailable` now supports `questions[]` extracted from `error.details.questions[]`, prompt contracts require this shape for missing facts, and runner pauses on `missing_required_input` with payload persisted under runtime agent context.
  - `AISRS-FT-004` (missing-input user interaction): in TTY mode runner asks user to resolve planner `questions[]` (option-select + custom value), writes answers into `runtime.inputs` and `fact_store` (`user_input` provenance), then continues with `plan.revise_segment` instead of terminating.
  - `AISRS-FT-005` (todo model + 状态机): runner introduces `TodoBoard` (`id/title/required_facts/produced_facts/acceptance/status/blocked_reason`) with explicit transitions `todo -> in_progress -> done|blocked`.
  - `AISRS-FT-006` (todo-first segmented loop): segmented intent execution now records/updates `runtime.agent.todo_progress`, scopes planner context with `state_summary.todo_state`, and host-enforces v1 `1 todo = 1 segment`.
  - `AISRS-FT-007` (write satisfiability gate templates): transfer/swap-like action segments are preflight-validated for gate chain (`query -> assert|branch -> action`), required query presence, and token decimals availability before compile/execute.
  - `AISRS-FT-008` (fact/todo checkpoint persistence): segmented mode checkpoint extensions now roundtrip `fact_store` + `todo_progress` and merge restored facts into initial runtime-derived facts on resume.
  - `AISRS-FT-009` (`plan.propose_todos`): segmented planner新增 `propose_todos` phase 与 `plan.propose_todos` finalize tool；runner 在无历史 todo_progress 时先规划 deterministic todos，再由 host 规范化并落地到 `runtime.agent.todo_progress`（失败时降级 bootstrap todo，不中断主流程）。
  - `AISRS-FT-010` (`todo_id` segment/receipt binding): runner 在执行前将 `todo_id` 绑定到 `segment.extensions.todo_id`，执行事件追加 `event.extensions.agent.{todo_id,segment_id,step_id}`，并将 round 级执行回执回写到 `todo_progress.todos[].receipt`（status/paused_reason/node_ids/completed_node_ids/tx_hashes/event_types/event_count）。
  - `AISRS-FT-011` (fact staleness + refresh): `FactStore` 增加 `stability/observed_at_ms` 元数据并区分 stable/volatile facts；写前门控对 volatile facts（balance/allowance）执行 freshness 检查，若未在同段 refresh query 且缓存过期则返回 `stale_volatile_fact`，强制先查后写。
  - `AISRS-FT-013` (orchestrator 模块化): segmented 主循环已拆分到 `agent/orchestrator.rs`，按 `plan_round -> compile_guard -> execute_round -> checkpoint_round` 阶段执行；`mod.rs` 仅保留入口装配。
  - `AISRS-FT-014` (context projection + diff): 新增 `agent/context_view.rs` 的 `PlanningContextManager`，统一 `state_summary` 投影并通过稳定哈希标注 `context_unchanged`，同时对 fact payload 做有界压缩。
  - `AISRS-FT-015` (checkpoint extensions typed codec): 新增 `agent/checkpoint_ext.rs` 统一 `planning_memory/fact_store/todo_progress` 编解码；segmented checkpoint 保存改为 typed `encode_updated`，并保留 unknown extensions 透传。
  - `AISRS-FT-016` (`stores` fact backfill): segmented 执行回调按 `step.stores` 从 `runtime.nodes.<node_id>.outputs` 映射并回填 `fact_store`（`Observed + QueryObserved + provenance`），回填结果随 checkpoint 持久化供后续轮次直接消费。
  - `AISRS-FT-017` (write-gate policy modularization): 写前门控已拆分到 `agent/write_gates.rs`，并移除基于 `candidate_ref/id` 的 `transfer|swap` 字符串启发式；改为结构化字段判定（`risk_tags/requires_queries/params`）+ `write_gate` 显式覆盖配置。
  - `AISRS-FT-018` (error state convergence): 新增 `agent/error_state.rs` 统一 planning/compile/execution 错误分类与 payload；planner 输出修复改为模式表驱动，compile 错误统一封装为 `phase=compile`，并收敛 `previous_error` 与 `state_summary` 更新路径。
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
