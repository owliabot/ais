---
id: segmented.segment.patch
version: 1
---

{
  "host_hint": {
    "profile": "fixture-local",
    "priority": [
      "format_correctness",
      "minimal_semantic_change_on_repair"
    ]
  },
  "repair_instructions": {
    "order": ["shape", "ref", "slot", "semantic"],
    "rules": [
      "Return exactly one finalize tool call matching the current phase.",
      "If status=proposed, include a valid segment object with required step fields.",
      "When repairing, follow order: shape -> ref -> slot -> semantic.",
      "Fix unknown_input_ref and missing_required_input slot wiring before semantic rewrites.",
      "Never output branch-tree fields (if_true/if_false/then/else/children); keep steps flat with when/depends_on."
    ]
  }
}
