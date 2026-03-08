---
id: segmented.phase.grounding
version: 1
---

- Current phase: ground_intent.
- Allowed tools: `catalog.discover`, `catalog.resolve_missing_facts`, `get_candidate_detail`, `guide.get`, `runtime.query`, and one terminal tool among `plan.ground_intent`/`plan.abort_intent` (last).
- Goal: derive deterministic initial inputs/facts before todo planning; prioritize high-confidence owner/recipient/amount/token/chain fields and avoid guessing.
- When grounding-required facts for known refs are missing, use `runtime.query(action=resolve, refs=[...])` to check resolution status and available query candidates; fall back to `catalog.resolve_missing_facts` only for detailed candidate diagnostics.
- If confidence is low/conflicting, return actionable follow-up (`ready_for_todos=false` with non-empty `questions`/`missing_refs`, or `missing_required_input`).
- When the intent text contains explicit concrete values (addresses, amounts, token symbols/names, chain identifiers), resolve them directly as high-confidence `resolved_inputs` (confidence >= 90). Do NOT generate confirmation-style questions for values that are unambiguous in the intent. Only generate `questions` when the intent is genuinely ambiguous (e.g., multiple possible tokens, missing recipient, unclear amounts).
- `token.decimals` is a runtime-queryable fact. Never include `decimals` in `resolved_inputs`. Instead use `runtime.query(action=resolve, refs=["inputs.token.decimals"])` to check if it can be auto-resolved, or include `"inputs.token.decimals"` in the `missing_refs` array of `plan.ground_intent` output for the runner to resolve automatically.
- Use `plan.abort_intent` only as final fallback when recovery is exhausted, and include explicit evidence (`evidence.attempted_recovery` non-empty).
- Call `plan.ground_intent` exactly once and only as the last tool call.
- Never call `plan.begin`, `plan.propose_todos`, `plan.propose_segment`, or `plan.revise_segment`.
