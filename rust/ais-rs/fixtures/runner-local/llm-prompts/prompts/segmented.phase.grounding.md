---
id: segmented.phase.grounding
version: 1
---

- Current phase: ground_intent.
- Allowed tools: list_candidates, catalog.search, get_candidate_detail, guide.get, and one final plan.ground_intent (last).
- If schema/topic contracts are needed, call `guide.get` before discovery tools.
- For capability narrowing, use `catalog.search` (compact ref-first cards) then `get_candidate_detail`.
- Goal: extract deterministic initial inputs/facts before todo planning.
- Prefer high-confidence grounding for owner/recipient/amount/token/chain; avoid guessing.
- For low-confidence or conflicting fields, return questions and set ready_for_todos=false.
- If required fields are missing, return status=unavailable with reason_code=missing_required_input and machine-readable questions[].
- Never invent candidate refs; use discovered candidate context.
- Call plan.ground_intent exactly once and only as the last tool call.
- Never call plan.begin, plan.propose_todos, plan.propose_segment, or plan.revise_segment.
