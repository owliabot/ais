---
id: segmented.base_rules
version: 1
---

- Tool-calling only. Emit schema-typed JSON only: use JSON bool/number (never quoted strings), and self-check tool/finalize args before sending.
- Check `state_summary.tool_memory_projection` first and reuse cached discovery/schema results; do not repeat identical discovery calls in the same snapshot scope.
- For schema/topic and control-step semantics, use `guide.get` with canonical shape (`{"schema":"ais-plan-sketch/0.1.0"}` / `{"topic":"cel"}`); schema lookups are digest-first and request `{"full":true}` only when digest is insufficient.
- Capability narrowing order: prefer `catalog.search` (compact ref-first cards) then `get_candidate_detail`; use `list_candidates` only as broad inventory when needed.
- `list_candidates` policy template (filter-first): start with exact `chain`; add `protocol` when hinted; broaden only when empty/insufficient in strict order `exact chain+protocol -> exact chain -> chain namespace wildcard`.
- `assert`/`branch`/`until`/`retry` are PlanSketch control semantics (not catalog candidates); `candidate_ref` is required for `query`/`action` and optional for `assert`/`branch` control steps.
- Plan against `state_summary.todo_state.current_todo` only: produce exactly one deterministic segment for that todo, keep `depends_on` within the same segment, and never use cross-segment refs like `seg_1/...`.
- For `inputs.*`, only use refs from `state_summary.input_registry.known_refs`; never invent candidate/protocol/action refs outside discovered context.
- For `unknown_input_ref` repair, preserve slot semantics: token/address params map to address-like refs (for example `*.address`), and `*.decimals` refs cannot substitute token/address slots.
- Segment safety: for transfer/swap writes, enforce same-segment gate chain `query -> assert|branch -> action`; refresh volatile write facts (for example balance/allowance) and query decimals before token writes when needed.
- Failure/repair contract: return `status=invalid|unavailable` with `error.reason_code`; for missing facts use `status=unavailable` + `error.reason_code=missing_required_input` + `error.details.questions[]`; repair in order `shape -> ref -> slot -> semantic`.
