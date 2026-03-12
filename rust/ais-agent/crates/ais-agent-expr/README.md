# ais-agent-expr

Purpose:
- provide the reduced-scope expression layer for `ais-agent`
- keep local CEL-style derivation and predicate logic separate from runtime domain aggregates

Public API entry points:
- `cel`

Dependencies on workspace crates:
- none

Current implementation status:
- reduced-scope CEL parser, evaluator, scope binding, and type checker implemented

Known gaps:
- no alternate expression engines
- not intended for planning or orchestration
