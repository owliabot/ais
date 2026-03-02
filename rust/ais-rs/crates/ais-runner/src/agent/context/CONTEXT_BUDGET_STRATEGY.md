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

2. Stage-based structural trimming:
- stages: `balanced`, `tight`, `minimal`
- each stage caps specific sections (`input_store`, `input_slots`, `input_registry`, `canonical_context`, `node_output_refs`, `capability_view`, `previous_error`).
- first stage whose estimated tokens fit is selected; else fallback to smallest stage.

Then pressure actions are applied:

- critical:
  - drop `input_slots.canonical_refs`
  - drop `capability_view.protocols`
  - compress `tool_memory_projection`
  - compress/drop heavy finalize error payload fields
- medium/light:
  - progressively lighter compaction

All decisions are recorded into `state_summary.context_budget` (`stage`, `pressure_mode`, `pressure_actions`, estimates).

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

Projection content is then structurally trimmed in `planning_memory.rs` to this token budget.

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
4. if pressure is not normal:
- run `prune_for_pressure(...)`
- write prune diagnostics
5. derive projection budget
6. build projection
7. write projection diagnostics
8. update `context.tool_memory_projection`

## 8. Observability

Memory-pressure diagnostics are exposed under:

- `runtime.agent.llm_usage.diagnostics`

Current fields:

- `memory_prune_runs`
- `memory_pruned_entries_total`
- `memory_pruned_by_tool`
- `memory_projection_budget_tokens`
- `memory_projection_estimated_tokens`
- `memory_projection_empty_due_to_pressure_total`

`state_summary.context_budget` separately records context-level stage/pressure decisions.

## 9. Notes / Limits

- `prune_for_pressure` currently receives `active_todo` and `phase` but does not yet differentiate behavior by them.
- pressure pruning is class-based and deterministic; semantic ranking is currently strongest for `guide.get` (`ais-plan-sketch`, `cel` highest).
