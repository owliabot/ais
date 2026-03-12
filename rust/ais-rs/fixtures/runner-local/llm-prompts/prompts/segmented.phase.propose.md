---
id: segmented.phase.propose
version: 3
---

- Current phase: propose_segment.
- Allowed tools: `catalog.discover`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, `runtime.query`, `plan.check_segment`, and one terminal tool among `plan.propose_segment`/`plan.abort_intent` (last).
- Host enforces `1 todo = 1 segment`; only plan for `state_summary.todo_state.current_todo`.
- Segment shape must stay flat: do not output legacy branch-tree fields (`if_true`, `if_false`, `then`, `else`, `children`); encode branch paths with flat steps + `when.cel` + `depends_on`.
- You must call `plan.check_segment` before finalize; finalize proposed output only when check result has `ok=true`.
- Treat `get_candidate_detail.semantic_hints` as host-owned truth for deployment bindings, prerequisite queries, composite lowering, and pack merge state; do not reconstruct those semantics from catalog prose or invent fallback lowering.
- If `plan.check_segment` or local reasoning shows missing write-required facts (`missing_token_decimals`) or stale volatile write evidence (`stale_volatile_fact`), revise by adding the required query/gate steps before the write when possible. Use `runtime.query(action=resolve, refs=[...])` to inspect resolution status and `catalog.resolve_missing_facts` for detailed candidate diagnostics; if resolver/host recovery still has viable candidates, continue recovery and do not emit `missing_required_input` yet.
- If compile/pause diagnostics expose `token_policy_signal`, repair the token input to satisfy pack policy; do not guess alternate symbols/addresses or bypass host token resolution.
- For follow-up writes after an earlier write, assume prior balance/allowance queries are no longer fresh unless the write is intentionally backed by explicit historical `nodes.<step>.outputs.*` references.
- If required facts remain missing after recovery exhaustion, return `missing_required_input` with canonical `error.details.questions[]` + `error.details.recovery_exhaustion{unresolved_refs[],reasons[],attempt_trace_id}` (never patch token/address slots with `*.decimals` refs).
- Use `plan.abort_intent` only as final fallback when recovery is exhausted, and include explicit evidence (`evidence.attempted_recovery` non-empty).
- Never emit user-facing `missing_required_input` refs/questions with `params.*`; use canonical source refs (`inputs.*` / node outputs) only.
- Call `plan.propose_segment` exactly once and only as the last tool call.
- Never call `plan.begin` or `plan.revise_segment`.
