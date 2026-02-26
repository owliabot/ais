---
id: segmented.phase.propose
version: 1
---

- Current phase: propose_segment.
- Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, plan.check_segment, and one final plan.propose_segment (last).
- Host enforces `1 todo = 1 segment`; only plan for `state_summary.todo_state.current_todo`.
- If schema/topic contracts are needed, call `guide.get` first.
- For capability narrowing, use `catalog.search` (compact ref-first cards) -> `get_candidate_detail`.
- Use `list_candidates` only as broad inventory when search context is not yet established.
- Prefer one list_candidates call per snapshot scope; reuse cached discovery results instead of duplicate calls.
- Never call get_candidate_detail for refs that were not discovered.
- `assert`/`branch`/`until`/`retry` semantics must be read from `guide.get`, not `catalog.search`.
- Even for `assert`/`branch` control steps, `candidate_ref` is required in `steps[]` and must be a discovered candidate ref.
- Do not output legacy branch-tree fields: `if_true`, `if_false`, `then`, `else`, `children`.
- Branch path must be encoded with flat steps plus `when.cel` and `depends_on`, not nested child-step trees.
- If uncertain about field names or contracts, call guide.get first with canonical shape `{ "schema":"..." }` or `{ "topic":"..." }`.
- Use `state_summary.input_registry.known_refs` as the source of truth for `inputs.*` refs; do not invent missing refs.
- You must call `plan.check_segment` before finalizing; finalize only when check result has `ok=true`.
- For transfer/swap actions, include gate chain `query -> assert|branch -> action` and wire dependencies accordingly.
- For volatile facts (balance/allowance), refresh via same-segment query before write; do not rely on stale context-only values.
- If token decimals are missing for token writes, add a decimals query step first (or return `missing_required_input` with questions).
- If compile returns `unknown_input_ref`, revise refs to entries from `state_summary.input_registry.known_refs` before other edits.
- Repair order is strict: `shape -> ref -> slot -> semantic`.
- If required facts are missing, finalize with `status=unavailable`, `error.reason_code=missing_required_input`, and machine-readable `error.details.questions[]`.
- Call plan.propose_segment exactly once and only as the last tool call.
- Never call plan.begin or plan.revise_segment.
