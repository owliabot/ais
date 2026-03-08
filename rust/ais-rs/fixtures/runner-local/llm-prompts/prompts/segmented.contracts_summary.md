---
id: segmented.contracts_summary
version: 3
---

- ValueRef forms: lit/ref/cel/object/array. Examples: `{"amount":{"lit":"10"}}`, `{"owner":{"ref":"inputs.owner"}}`, `{"ok":{"cel":"nodes.q_balance.outputs.balance > to_atomic(100, inputs.token.decimals)"}}`. In CEL, use host-exposed runtime roots only. Common runtime roots in segmented planning are `inputs`, `params`, and `nodes`; do not invent new roots. Node refs: `nodes.<step_id>.outputs.<field>` (same-segment step ids only). Common helpers include `to_atomic`, `to_human`, `size`, `contains`, `int`, and `string`; use `guide.get(topic="cel")` for the exact CEL helper/root contract when needed.
- Segment contract: step required fields: id, kind, inputs. kind enum: action/query/assert/branch. `candidate_ref` required for query/action, optional for assert/branch. `depends_on` references step ids in the same segment only. Forbidden legacy keys: if_true/if_false/then/else/children. Branch encoding: flat steps with `when.cel` + `depends_on`.
- Asset param contract: for param type=asset, input must resolve to object with address. Preferred shape: `{"object":{"address":{"lit":"0x..."},"chain_ref":{"lit":"eip155:..."}}}`. Canonical decimals source is the leaf ref `inputs.<asset>.decimals` or a resolved asset object with numeric `decimals`. Compiler normalizes chain_ref to chain_id.
- Write gate contract: transfer/swap action steps require `action -> assert|branch` gating. Gate backing may come from same-segment query ancestry or explicit historical `nodes.<step>.outputs.*` references. Use `depends_on` for same-segment scheduling/gate reachability only; do not invent query deps when reading stable historical node outputs.
- Freshness contract: `balance` / `allowance` are volatile query facts. If a write depends on them and there is no explicit historical node-output backing, add a fresh query in the same segment before the write. After a successful write, prior volatile observations are invalidated; follow-up writes must query again.
- Decimals contract: use `inputs.token.decimals` or another canonical `inputs.<asset>.decimals` leaf in CEL (`to_atomic(..., inputs.token.decimals)`). Do not use `*.decimals` refs as substitutes for token/address params.
- Use CEL for deterministic conditions and value computation; expressions must be side-effect free.
- For typing/schema lookup contracts, use `guide.get(topic="typing")`. For failure/error shape contracts, use `guide.get(topic="failure")`.
