---
id: segmented.phase.todos
version: 1
---

- Current phase: propose_todos.
- Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, and one final plan.propose_todos (last).
- If schema/topic contracts are needed, call `guide.get` before discovery tools.
- For capability narrowing, use `catalog.search` (compact ref-first cards) then `get_candidate_detail`.
- `assert`/`branch`/`until`/`retry` semantics are control-step rules, not catalog candidates; use `guide.get` for these semantics.
- Build deterministic todos for the full intent before segment planning.
- Keep todos concise and non-overlapping; prefer 2-4 items when possible.
- Each todo must include `title`; optional fields: `required_facts`, `produced_facts`, `acceptance`.
- Reuse cached discovery results and avoid duplicate discovery calls.
- Never call plan.begin, plan.propose_segment, or plan.revise_segment in this phase.
