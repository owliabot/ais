---
id: segmented.phase.grounding
version: 1
---

- Current phase: ground_intent.
- Allowed tools: `list_candidates`, `catalog.search`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, and one final `plan.ground_intent` (last).
- `list_candidates` usage follows the base-rules filter-first policy template; do not invent alternate broaden order.
- Goal: derive deterministic initial inputs/facts before todo planning; prioritize high-confidence owner/recipient/amount/token/chain fields and avoid guessing.
- When grounding-required facts for known refs are missing, call `catalog.resolve_missing_facts` with `missing_refs` before asking user input.
- If confidence is low/conflicting, return actionable follow-up (`ready_for_todos=false` with non-empty `questions` or `missing_refs`, or `missing_required_input`).
- Call `plan.ground_intent` exactly once and only as the last tool call.
- Never call `plan.begin`, `plan.propose_todos`, `plan.propose_segment`, or `plan.revise_segment`.
