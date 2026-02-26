# AIS-2J: Compiler Issues Contract (`ais-compiler-issues/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

This document defines a stable machine-readable issue contract for:

- sketch validation failures
- compile-time planning failures
- policy preflight failures during compile/handoff

It is designed to be consumed by:

- segmented planner loops (`plan.propose_segment` / `plan.revise_segment`)
- runner diagnostics
- UI and test harnesses

---

## 1. Purpose and boundaries

Compiler issues are not free-text logs.
They are structured records that support deterministic automation and LLM repair loops.

This contract applies to compile/handoff stages.
Runtime execution failures continue to use engine event/error contracts and may be projected into this shape when fed back to planning.

---

## 2. Root shape (`ais-compiler-issues/0.1.0`)

Required:

- `schema`: MUST be `ais-compiler-issues/0.1.0`
- `issues`: non-empty array of issue records

Optional:

- `summary`: short aggregate message
- `snapshot`: `{ pack_snapshot_hash?, catalog_hash?, plan_epoch? }`
- `extensions`

Normative rules:

- unknown top-level fields MUST be rejected.
- issue ordering SHOULD be deterministic:
  - primary: `severity` (`error` before `warning`)
  - secondary: `reason_code`
  - tertiary: `field_path`

---

## 3. Issue record shape

Required fields:

- `kind`: `schema_error|compile_error|policy_error|runtime_error`
- `severity`: `error|warning`
- `reason_code`: stable machine code
- `message`: human-readable short message

Optional fields:

- `field_path`: field location (see section 4)
- `node_id`: target node/step id if available
- `candidate_ref`: related candidate ref if available
- `suggestion`: short machine-friendly fix hint
- `details`: implementation-specific object
- `reference`: source reference (`json_schema.validation`, `pack.allowlist`, etc.)

Normative rules:

- `reason_code` MUST be snake_case and stable across patch releases.
- `suggestion` SHOULD be action-oriented and short (for example: `add inputs.owner`).

---

## 4. `field_path` normalization

`field_path` MUST be represented as JSON Pointer (RFC 6901 style), for example:

- `/segments/0/steps/1/inputs/owner`
- `/segments/0/steps/0/candidate_ref`

Rules:

- root path MUST be `/`
- array indices MUST be numeric path segments
- implementers MAY also keep an internal typed field-path structure, but exported contract MUST use JSON Pointer

Mapping guidance for Rust `StructuredIssue`:

- `FieldPath::root()` -> `/`
- key segment -> `/key`
- index segment -> `/<index>`

---

## 5. Stable reason codes (minimum required set)

Implementations MUST support at least:

- `candidate_not_found`
- `candidate_chain_not_allowed`
- `execution_type_not_allowed`
- `missing_required_input`
- `input_type_mismatch`
- `constraint_violation`
- `policy_requires_confirm`
- `segment_mutation_not_allowed`
- `planner_round_limit_reached`

Implementations MAY add more reason codes, but:

- new codes MUST be documented
- existing code semantics MUST NOT drift silently

---

## 6. Planner integration rules

When compile/handoff fails in segmented planning:

1) host SHOULD emit `ais-compiler-issues/0.1.0`
2) host SHOULD pass issues into `plan.revise_segment.previous_error.issues`
3) planner SHOULD prioritize fixes for `severity=error`

If `policy_requires_confirm` appears:

- host MAY pause for confirmation instead of revise loop, based on active approval mode.

---

## 7. Authority schema

- JSON Schema: `schemas/0.0.2/compiler-issues.schema.json`

