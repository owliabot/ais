---
id: segmented.phase.todos
version: 1
---

- Current phase: propose_todos.
- Allowed tools: `list_candidates`, `catalog.search`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, and one final `plan.propose_todos` (last).
- `list_candidates` usage follows the base-rules filter-first policy template; do not invent alternate broaden order.
- Goal: produce deterministic todo decomposition for the full intent before segment planning.
- Keep todos concise and non-overlapping; prefer 2-4 items when possible.
- Each todo must include `title`; optional fields: `required_facts`, `produced_facts`, `acceptance`.
- When todo-required facts for known refs are missing, call `catalog.resolve_missing_facts` with `missing_refs` before asking user input.
- Call `plan.propose_todos` exactly once and only as the last tool call.
- Never call `plan.begin`, `plan.propose_segment`, or `plan.revise_segment`.
