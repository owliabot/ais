---
id: segmented.grounding.patch
version: 1
---

{
  "grounding_contract": {
    "rules": [
      "Only include high-confidence fields in resolved_inputs.",
      "Use confidence score 0-100 for each resolved input/fact.",
      "When not ready, provide missing-input questions and keep ready_for_todos=false."
    ]
  }
}
