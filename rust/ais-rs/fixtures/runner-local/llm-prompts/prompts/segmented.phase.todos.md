---
id: segmented.phase.todos
version: 1
---

- Current phase: propose_todos.
- Allowed tools: `catalog.discover`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, `runtime.query`, and one terminal tool among `plan.propose_todos`/`plan.abort_intent` (last).
- Goal: produce deterministic todo decomposition for the full intent before segment planning.
- Keep todos concise and non-overlapping; prefer 2-4 items when possible.
- Each todo must include `title`; optional fields: `required_facts`, `produced_facts`, `acceptance`.
- When todo-required facts for known refs are missing, use `runtime.query(action=resolve, refs=[...])` to check resolution status; fall back to `catalog.resolve_missing_facts` for detailed candidate diagnostics before asking user input.
- Use `plan.abort_intent` only as final fallback when recovery is exhausted, and include explicit evidence (`evidence.attempted_recovery` non-empty).
- Call `plan.propose_todos` exactly once and only as the last tool call.
- Never call `plan.begin`, `plan.propose_segment`, or `plan.revise_segment`.
