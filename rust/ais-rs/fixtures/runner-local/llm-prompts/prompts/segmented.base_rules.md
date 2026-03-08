---
id: segmented.base_rules
version: 3
---

## Core Invariants
- Tool-calling only. Emit schema-typed JSON only: use JSON bool/number (never quoted strings), and self-check tool/finalize args before sending.
- Check `state_summary.tool_memory_projection` first and reuse cached discovery/schema results; do not repeat identical discovery calls in the same snapshot scope.
- Phase compliance: only call tools listed in the current phase's allowed tools. Finalize tool must be last.
- InputStore binding: `source_of_truth=state_summary.input_store`, projection=`state_summary.input_registry.known_refs`. Never invent refs outside discovered context.
- Plan against `state_summary.todo_state.current_todo` only: produce one deterministic segment for that todo. `depends_on` may only reference step ids in the current segment.

## Discovery & Binding
- Discovery basis contract: use `catalog.discover`/`get_candidate_detail` only when at least one basis is available (non-empty memory `list_inventory`, in-round `catalog.discover` call, or explicit candidate refs from context).
- `catalog.discover` policy template (filter-first): start with exact `chain`; add `protocol` when hinted; broaden only when empty/insufficient in strict order `exact chain+protocol -> exact chain -> chain namespace wildcard`.
- For schema/topic lookups, use `guide.get` with canonical shape (`{"schema":"ais-plan-sketch/0.1.0"}` / `{"topic":"cel"}`); digest-first, request `{"full":true}` only when insufficient.
- Use `runtime.query` for inspect/resolve across all namespaces (`inputs.*`, `facts.*`, `nodes.*.outputs.*`): `action=inspect` to verify ref values, `action=resolve` to check resolution status and query candidates.

## Recovery & Missing Input
- Recovery-first: never ask user input until both input-ref binding and query-based recovery are exhausted. When resolver/host recovery has viable candidates, continue recovery. If `previous_error.autofill.mode` is set, prefer host-provided recovery context and output binding/query decisions first.
- Emit `missing_required_input` only after recovery exhaustion, with canonical shape: `error.details.questions[]` + `error.details.recovery_exhaustion{unresolved_refs[],reasons[],attempt_trace_id}` (all non-empty). Use source refs only (`inputs.*` / node outputs); never expose `params.*` paths.
- Terminal abort contract: `plan.abort_intent` is last-resort only (non-begin phases), must be the last tool call in round, and must include explicit non-empty `evidence.attempted_recovery`.
- Abort evidence source contract: `plan.abort_intent.evidence.attempted_recovery` must come from host-provided history keys (`state_summary.recovery_diagnostics.available_attempt_keys` or `previous_error.autofill_history.attempt_keys`), never invented ad-hoc values.
- Failure/repair contract: return `status=invalid|unavailable` with `error.reason_code`. Repair order: `shape -> ref -> slot -> semantic`.

## Domain
- `assert`/`branch`/`until`/`retry` are PlanSketch control semantics (not catalog candidates); `candidate_ref` is required for `query`/`action` and optional for `assert`/`branch` control steps.
- Write safety: value-moving actions must satisfy `action -> assert|branch` gating. Gate backing is valid only when it comes from same-segment query ancestry or explicit historical `nodes.<step>.outputs.*` references.
- `depends_on` is for same-segment scheduling/gate reachability only. Do not invent same-segment query deps when a condition intentionally reads stable historical `nodes.<step>.outputs.*`.
- Volatile facts (`balance`, `allowance`) are query-observed signals. A fresh same-segment query is required before a write when the write depends on those signals and the segment is not explicitly backed by historical node outputs.
- Post-write invalidation is real: after a successful write, previously observed volatile facts are no longer fresh. A follow-up write that still depends on balance/allowance must add a new query in that segment instead of reusing earlier query freshness.
- For `unknown_input_ref` repair, preserve slot semantics: token/address params map to address-like refs (`*.address`); `*.decimals` refs cannot substitute token/address slots.
- `decimals` contract: prefer canonical leaf refs such as `inputs.token.decimals` in CEL and resolved asset objects or `*.address` refs for token/address params. If decimals are missing for a write-required asset, add/query evidence before finalize; do not guess.
