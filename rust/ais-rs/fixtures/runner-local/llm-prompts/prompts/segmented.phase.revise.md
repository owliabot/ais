---
id: segmented.phase.revise
version: 1
---

- Current phase: revise_segment.
- Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, plan.check_segment, and one final plan.revise_segment (last).
- Keep repairing the same `state_summary.todo_state.current_todo`; do not switch objectives.
- If schema/topic contracts are needed, call `guide.get` first.
- For capability narrowing, use `catalog.search` (compact ref-first cards) -> `get_candidate_detail`.
- Use `list_candidates` only as broad inventory when search context is not yet established.
- Prefer one list_candidates call per snapshot scope; reuse cached discovery results instead of duplicate calls.
- Never call get_candidate_detail for refs that were not discovered.
- `assert`/`branch`/`until`/`retry` semantics must be read from `guide.get`, not `catalog.search`.
- Even for `assert`/`branch` control steps, `candidate_ref` is required in `steps[]` and must be a discovered candidate ref.
- Do not output legacy branch-tree fields: `if_true`, `if_false`, `then`, `else`, `children`.
- Branch path must be encoded with flat steps plus `when.cel` and `depends_on`, not nested child-step trees.
- Apply minimum edits to fix tool output shape; keep plan semantics stable.
- If `previous_error.last_failed_finalize` exists, use it as baseline draft and patch minimally instead of regenerating from scratch.
- If unsure about contracts, call guide.get with canonical shape `{ "schema":"..." }` or `{ "topic":"..." }` instead of guessing.
- Use `state_summary.input_registry.known_refs` as the source of truth for `inputs.*` refs; replace guessed refs first.
- You must call `plan.check_segment` before finalizing; finalize only when check result has `ok=true`.
- Preserve transfer/swap write gates: `query -> assert|branch -> action` within the same segment.
- Ensure volatile facts (balance/allowance) are refreshed by same-segment query steps before write.
- If token decimals are missing for token writes, add decimals query steps or return `missing_required_input` with `questions[]`.
- If compile returns `unknown_input_ref`, revise refs to known registry refs before semantic rewrites.
- Repair order is strict: `shape -> ref -> slot -> semantic`; keep semantic edits minimal.
- If required facts are missing, finalize with `status=unavailable`, `error.reason_code=missing_required_input`, and machine-readable `error.details.questions[]`.
- Call plan.revise_segment exactly once and only as the last tool call.
- Never call plan.begin or plan.propose_segment.
