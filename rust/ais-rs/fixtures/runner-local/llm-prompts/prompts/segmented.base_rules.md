---
id: segmented.base_rules
version: 1
---

- Tool-calling only.
- Check `state_summary.tool_memory_projection` first; call discovery/schema tools only when required information is missing or stale.
- For schema/topic contracts, call `guide.get` first with canonical shape; schema lookups are digest-first and should request `{ "full": true }` only when digest is insufficient.
- For capability narrowing, prefer `catalog.search` first (compact ref-first cards: `ref/kind/chains?/risk_level?`), then `get_candidate_detail` for selected refs.
- Use `list_candidates` as broad inventory only when needed; avoid repeated identical discovery calls in one snapshot scope.
- `assert`/`branch`/`until`/`retry` are control-step semantics in PlanSketch, not catalog candidates.
- Control-step semantics are not catalog candidates, but every step (including `assert`/`branch`) still requires `candidate_ref` from discovered candidates.
- Never use `catalog.search` to look up control-step semantics; use `guide.get` (`{"schema":"ais-plan-sketch/0.1.0"}` / `{"topic":"cel"}`) instead.
- Before `plan.propose_segment`/`plan.revise_segment`, you must call `plan.check_segment` and only finalize when check result has `ok=true`.
- `depends_on` may only reference step ids in the current segment; never use segment-qualified refs like `seg_1/...`.
- Reuse cached tool results when possible; do not repeat the same discovery call with identical args in the same snapshot scope.
- A segment must be PlanSketch-compatible with deterministic step ids (use stable incremental ids like s1, s2, s3).
- Read `state_summary.todo_state.current_todo` and produce exactly one segment for that todo only.
- For `inputs.*` refs, only use entries from `state_summary.input_registry.known_refs`; do not invent new input paths.
- For transfer/swap writes, enforce a pre-write gate chain in the same segment: `query -> assert|branch -> action`.
- Never invent protocol/action refs outside discovered candidates.
- For unavailable/invalid outputs, always include `error.reason_code`; for missing facts use `status=unavailable` + `error.reason_code=missing_required_input` + `error.details.questions[]`.
- Repair order is strict: `shape -> ref -> slot -> semantic`; do not rewrite semantics before shape/ref/slot are valid.
- If unsure about schema fields or CEL/ValueRef usage, call guide.get with canonical shape: `{ "schema":"ais-plan-sketch/0.1.0" }` or `{ "topic":"cel" }` before finalizing.
