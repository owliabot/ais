---
id: segmented.phase.revise
version: 2
---

- Current phase: revise_segment.
- Allowed tools: `catalog.discover`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, `runtime.query`, `plan.check_segment`, and one terminal tool among `plan.revise_segment`/`plan.abort_intent` (last).
- Keep repairing the same `state_summary.todo_state.current_todo`; do not switch objectives.
- Apply minimum edits and keep semantics stable; when available, patch `previous_error.last_failed_finalize` instead of regenerating from scratch.
- Segment shape must stay flat: do not output legacy branch-tree fields (`if_true`, `if_false`, `then`, `else`, `children`); encode branch paths with flat steps + `when.cel` + `depends_on`.
- CRITICAL: Before calling `plan.revise_segment`, you MUST first call `plan.check_segment` with the segment you intend to finalize. The correct sequence is: (1) call `plan.check_segment` with the segment, wait for `ok=true`; (2) only then call `plan.revise_segment` with the checked segment. Calling revise without a preceding successful check will fail.
- If `plan.check_segment` reports `missing_token_decimals` or `stale_volatile_fact`, repair the segment by adding the required query/gate steps before the write when possible. Use `runtime.query(action=resolve, refs=[...])` to inspect resolution status and `catalog.resolve_missing_facts` for detailed candidate diagnostics; if resolver/host recovery still has viable candidates, continue recovery and do not emit `missing_required_input` yet.
- For follow-up writes after an earlier write, assume prior balance/allowance queries are no longer fresh unless the write is intentionally backed by explicit historical `nodes.<step>.outputs.*` references.
- If required facts remain missing after recovery exhaustion, return `missing_required_input` with canonical `error.details.questions[]` + `error.details.recovery_exhaustion{unresolved_refs[],reasons[],attempt_trace_id}` (never patch token/address slots with `*.decimals` refs).
- Use `plan.abort_intent` only as final fallback when recovery is exhausted, and include explicit evidence (`evidence.attempted_recovery` non-empty).
- Never emit user-facing `missing_required_input` refs/questions with `params.*`; use canonical source refs (`inputs.*` / node outputs) only.
- Repair order is strict: `shape -> ref -> slot -> semantic`; keep semantic edits minimal.
- Call `plan.revise_segment` exactly once and only as the last tool call.
- Never call `plan.begin` or `plan.propose_segment`.
