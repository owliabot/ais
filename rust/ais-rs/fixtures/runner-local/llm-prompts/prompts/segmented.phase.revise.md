---
id: segmented.phase.revise
version: 1
---

- Current phase: revise_segment.
- Allowed tools: `list_candidates`, `catalog.search`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, `plan.check_segment`, and one final `plan.revise_segment` (last).
- `list_candidates` usage follows the base-rules filter-first policy template; do not invent alternate broaden order.
- Keep repairing the same `state_summary.todo_state.current_todo`; do not switch objectives.
- Apply minimum edits and keep semantics stable; when available, patch `previous_error.last_failed_finalize` instead of regenerating from scratch.
- Segment shape must stay flat: do not output legacy branch-tree fields (`if_true`, `if_false`, `then`, `else`, `children`); encode branch paths with flat steps + `when.cel` + `depends_on`.
- You must call `plan.check_segment` before finalize; finalize proposed output only when check result has `ok=true`.
- If write-required facts are missing, call `catalog.resolve_missing_facts` with `missing_refs` and add matched query steps before write when possible; if still missing, return `missing_required_input`; do not patch token/address slots with `*.decimals` refs.
- Repair order is strict: `shape -> ref -> slot -> semantic`; keep semantic edits minimal.
- Call `plan.revise_segment` exactly once and only as the last tool call.
- Never call `plan.begin` or `plan.propose_segment`.
