# AIS Runner Context Budget Strategy

## Scope

This document explains the current dynamic context-budget and memory-budget strategy used by:

- `context/budget_policy.rs`
- `context/budgeter.rs`
- `orchestrator.rs::refresh_tool_memory_projection`
- `planning_memory.rs`

The goal is to keep planner context stable under normal load and degrade predictably near context-window limits.

All major limits now converge to one policy module (`context/budget_policy.rs`), and consumers (`budgeter`, `planning_memory`, `tools/dispatch`) read from this policy instead of hardcoding local constants.

## 1. Input Signals

Budget decisions use runtime/planner usage fields:

- `context_window_input_tokens`
- `context_remaining_tokens`
- `context_soft_limit_tokens`

Key derived signals:

- `remaining_ratio_bps = remaining / soft_limit * 10000`
- `usage_ratio_bps = 10000 - remaining_ratio_bps`
- when both window-based and remaining-based usage exist, strategy uses the more conservative usage value (`max`).

## 2. Pressure Mode

`ContextPressureMode` has four levels:

- `normal`
- `light`
- `medium`
- `critical`

Current thresholds (single source in `budget_policy.rs`):

- usage thresholds:
  - light: `>= 7000 bps`
  - medium: `>= 8500 bps`
  - critical: `>= 9200 bps`
- absolute remaining-token guards:
  - medium floor: `remaining <= 8000`
  - critical floor: `remaining <= 3000`

Mode selection is worst-case: crossing either usage or remaining guard escalates pressure.

## 3. Planner `state_summary` Budgeting

`context/budgeter.rs` performs two layers:

1. Adaptive token limit:
- base token limit comes from planner config/CLI.
- adaptive output limit scales by usage:
  - relaxed (< light): up to ~1.5x base (capped by `ADAPTIVE_RELAXED_MAX_MULTIPLIER`)
  - balanced: base
  - medium: `17/20 * base`
  - tight: `3/5 * base`

2. Single pack loop over optional blocks (`pack_blocks(...)`):
- default behavior: when budget allows, keep full projected context.
- under pressure: progressively compress low/stale optional blocks first, then evict them if still over budget.
- must-keep core is never evicted; if the must-keep-only remainder still exceeds budget, an explicit overflow signal is emitted.

Context decisions are recorded under `state_summary.context_budget` with a minimal surface:

- `pressure_mode`
- `pack_trace[]`: ordered decisions (`keep|compress|drop`) with `block_id`, level transitions, and reason codes
- `pack_diagnostics`: stable counters for `packed/compressed/evicted` with reason breakdowns
- `pack_overflow_reason` (nullable; present when overflow occurred)
- `final_compact_applied` (`true` only when overflow fallback compact ran)

## 4. Tool-Memory Projection Budget

`ToolMemoryBudgetPolicy::derive_tool_memory_projection_token_budget(...)`:

- when `soft_limit` is available:
  - dynamic bounds are derived from soft limit:
    - lower bound: `20% * soft_limit`
    - upper bound: `40% * soft_limit`
  - current remaining-ratio mapping still applies:
    - <= 20% remaining -> lower bound
    - >= 60% remaining -> upper bound
    - linear interpolation in between
- fallback mapping (when no soft limit): absolute remaining
  - <= 4000 -> min
  - >= 24000 -> max
  - linear interpolation in between
- no signal -> default `2400`
- absolute safety clamp remains in projection normalization:
  - min `1200`
  - max `64000`

Projection content is produced as candidates (`full` / `summary` / `skeleton`) under this budget signal, then selected by global compress level.

Projection per-bucket caps are also dynamic now:

- `list_inventory`
- `catalog_search`
- `candidate_detail`
- `guide`

Caps are derived from the projection budget (smaller budget -> fewer entries, larger budget -> more entries).

## 5. PlanningMemory Store Budget (Dynamic)

`ToolMemoryBudgetPolicy::derive_planning_memory_store_budget(...)` derives store caps:

- baseline interpolation envelope:
  - `max_entries`: `16..72`
  - `max_entry_chars`: `3000..10000`
  - `max_total_chars`: `40000..180000`
- interpolation progress follows projection-budget progress (same headroom signal path).
- pressure-mode caps enforce hard upper bounds:
  - critical: `<= 16 / 3000 / 40000`
  - medium: `<= 32 / 6000 / 80000`
  - light: `<= 48 / 8000 / 120000`
- if no usage signal exists, default to `48 / 8000 / 120000`.

`orchestrator::refresh_tool_memory_projection` applies this budget each refresh via:

- `planner.set_planning_memory_budget(...)`

`PlanningMemory::insert(...)`, checkpoint save, and restore all use current budget.

Pressure pruning keep-limits are also budget-aware now:

- keep counts are derived from projection budget + pressure mode
- critical/medium/light use stronger keep reduction multipliers

## 6. Tool Dispatch Compaction (Dynamic)

`tools/dispatch.rs` no longer keeps fixed per-tool JSON compact constants.

It derives a compact profile from projection budget:

- `tight`
- `balanced`
- `relaxed`

and fetches per-tool compaction options from policy:

- candidate detail
- resolve missing facts
- guide schema full
- guide schema digest
- guide topic
- check segment

So tool payload compaction follows the same context pressure signal path.

## 7. Runtime Flow (Per Refresh)

In `refresh_tool_memory_projection(...)`:

1. read usage
2. derive/apply store budget
3. resolve pressure mode
4. derive projection budget
5. build projection candidates (`full` / `summary` / `skeleton`)
6. select projection by global compress level
7. write projection diagnostics
8. update `context.tool_memory_projection`

## 8. Observability

Memory-pressure diagnostics are exposed under:

- `runtime.agent.llm_usage.diagnostics`

Current fields:

- `memory_projection_budget_tokens`
- `memory_projection_estimated_tokens`

`state_summary.context_budget` is intentionally compact and no longer carries payload/emitted token estimate compatibility aliases.

Pack-loop diagnostics are exposed under:

- `state_summary.context_budget.pack_diagnostics`
- `state_summary.context_budget.pack_trace[]`

## 9. Prompt Compact View

Planner prompt injection no longer sends full `state_summary` by default.

- `context_view` now emits `state_summary.prompt_compact` (`ais-agent-state-summary-prompt-compact/0.0.1`).
- `intent_segmented` prompt renderers (`todos` / `grounding` / `segment`) prefer `state_summary.prompt_compact`; they only fall back to full `state_summary` when compact view is absent.

Compact-view rules:

- keep machine-critical sections (`todo_state`, `input_registry`, `input_slots`, `canonical_context`, `intent_*`, `input_store`, `node_output_refs`, `tool_memory_projection`, `previous_error`)
- keep only minimal context-budget fields for prompt reasoning:
  - `pressure_mode`
  - `pack_overflow_reason`
  - compact `pack_diagnostics` counters
- drop `pack_trace` from prompt payload (while still keeping it in full `state_summary.context_budget` for observability)
- add a short `summary_text` line for quick model orientation

Input source-of-truth note:

- `InputStore` is the only input truth source for planning/runtime input refs.
- `input_slots` / `input_registry` / `canonical_context` are runtime-derived views from `InputStore`.
- `intent_slots` remains a grounding intermediate projection only (`input_binding.bindable=false`) and is not used as an additional input-ref source.

## 10. No-ToolCall Self-Recovery

In segmented planner rounds, empty `tool_calls` from model/provider is now treated as retryable planner output (bounded):

- runner injects a structured repair payload (`reason_code=no_tool_calls`) containing:
  - phase
  - expected finalize tool
  - allowed tools for current phase
  - retry attempt metadata
- retry limit: 2 (`NO_TOOLCALL_RETRY_ATTEMPT_LIMIT`)
- on exhaustion, runner returns a structured terminal error (`no_tool_calls_retries_exhausted`)

Diagnostics (under `runtime.agent.llm_usage.diagnostics`):

- `no_toolcall_retries_total`
- `no_toolcall_retries_exhausted_total`

## 11. Notes / Limits

- `planning_memory` no longer performs pressure pre-prune during projection refresh; it only enforces store budget and provides projection candidates.
