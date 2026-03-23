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
- builtin surface includes exact unit conversion helpers `to_atomic` and `to_unit`
- integer coercion is available via `int(...)`

Known gaps:
- no alternate expression engines
- not intended for planning or orchestration

Notes:
- `to_atomic(value, decimals)` performs exact unit-to-atomic scaling and fails on excess fractional precision
- `to_unit(value_atomic, decimals)` performs the reverse conversion and returns a CEL numeric value
