# AIS-2L: Constraint Templates and CEL Scope (`ais-constraint-templates/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

This document defines the canonical constraint-template mechanism for vNext planning.

Goal:

- avoid requiring LLMs to author raw CEL for safety-critical policy checks
- keep constraints auditable, reusable, and deterministic

It complements:

- Policy gate: `specs/ais-2-policy-gate.md`
- Plan sketch: `specs/ais-2-plan-sketch.md`
- Agent planning tools: `specs/ais-2-agent-planning.md`
- Expressions/CEL baseline: `specs/ais-1-expressions.md`

---

## 1. Model

Constraint templates are declared by pack/policy and referenced by planner output.

Two-stage flow:

1) declare template library (name, effect, CEL, parameter schema)
2) reference template by name with concrete params in sketch segment steps

Host/compiler resolves references deterministically and evaluates policy-gate decisions.

---

## 2. Template library contract (`ais-constraint-templates/0.1.0`)

Required top-level fields:

- `schema`: MUST be `ais-constraint-templates/0.1.0`
- `templates[]`: non-empty list

Template required fields:

- `name`: stable id (`snake_case`)
- `effect`: `hard_block|need_user_confirm`
- `expr`: CEL expression
- `param_schema`: JSON Schema object for template params

Optional template fields:

- `message`: human-readable message
- `severity`: `error|warning` (default `error`)
- `reason_code`: stable reason code override
- `extensions`

Normative rules:

- unknown fields MUST be rejected.
- template `name` MUST be unique in one library.
- `param_schema` MUST be strict enough to validate all required params.

---

## 3. Template references from planning

Planning step may include:

```json
{
  "constraint_templates": [
    { "name": "max_spend", "params": { "amount_atomic": "1000000" } }
  ]
}
```

Reference resolution rules:

- every reference `name` MUST exist in active template library.
- `params` MUST validate against template `param_schema`.
- unresolved template references MUST produce `constraint_violation` issue (or `missing_required_input` if params are incomplete).

---

## 4. CEL evaluation scope (normative)

Template CEL expressions are evaluated against a read-only context object with exactly these roots:

- `input`: normalized `PolicyGateInput`
- `params`: validated template params
- `policy`: active policy summary (mode, thresholds, allowlists summary)

No other root objects are allowed.

Explicitly forbidden roots:

- `nodes`
- `runtime`
- `env`
- network/IO handles

Determinism rules:

- CEL evaluation MUST be pure and side-effect-free.
- expression result MUST be deterministic for same `input + params + policy`.
- non-deterministic helpers (time-now/random/network) MUST NOT be available.

---

## 5. CEL built-in profile

Hosts SHOULD allow only a minimal deterministic built-in set:

- logical/comparison: `&&`, `||`, `!`, `==`, `!=`, `<`, `<=`, `>`, `>=`
- numeric/string/list primitives from baseline CEL
- explicit numeric conversion helpers where required (`uint`, `int`)

Hosts MUST NOT allow:

- dynamic eval/reflection
- function dispatch from untrusted strings
- host callbacks that perform external IO

---

## 6. Decision merge and precedence

Constraint result merge order:

1) policy-gate missingness base checks (`hard_block_fields/missing_fields/unknown_fields`)
2) matched template constraints in list order
3) remaining host policy checks

Precedence:

- any matched `hard_block` template wins
- else if any matched `need_user_confirm` template exists, decision is `need_user_confirm`
- else continue

Policy gate output SHOULD include:

- `matched_constraints[]` with template names
- `violations[]` with `name/effect/reason_code/message`

---

## 7. Recommended default template set (minimum)

Packs SHOULD provide at least:

- `max_spend`
- `max_slippage_bps`
- `disallow_unlimited_approval`
- `allowlist_tokens`

These are the default safety primitives for intent-mode demo flows.

---

## 8. Authority schema

- JSON Schema: `schemas/0.0.2/constraint-templates.schema.json`

