---
id: segmented.contracts_summary
version: 1
---

- ValueRef forms: lit/ref/cel/object/array.
- Asset shape: object.address + object.chain_ref (compiler normalizes to chain_id).
- Use CEL for deterministic conditions and value computation; expressions must be side-effect free.
- Express write-safety with deterministic CEL conditions plus explicit query/assert/branch guards.
