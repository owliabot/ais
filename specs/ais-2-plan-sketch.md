# AIS-2H: Plan Sketch Contract (`ais-plan-sketch/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

`PlanSketch` is the LLM-facing planning IR for intent mode.
It is intentionally smaller and more stable than `ais-plan/0.0.3`.

This contract is designed for segmented planning:

- model proposes one segment at a time
- host compiles segment to executable plan nodes deterministically
- engine executes and returns structured state/error for the next segment

---

## 1. Purpose and boundaries

`PlanSketch` MUST be used as the planner output contract in vNext intent mode.

`PlanSketch` is NOT executable by the engine directly.
The host MUST compile `PlanSketch` into `ais-plan/0.0.3` before execution.

`PlanSketch` intentionally excludes low-level execution fields:

- no `execution.type/method/params`
- no `writes` path details
- no engine node runtime fields

Those fields MUST be filled by deterministic compiler logic using candidates + pack + runtime capabilities.

---

## 2. Root shape (`ais-plan-sketch/0.1.0`)

Required root fields:

- `schema`: MUST be `ais-plan-sketch/0.1.0`
- `intent`: non-empty user intent text
- `pack_snapshot`: pack identity/hash used during planning
- `catalog_snapshot`: catalog identity/hash used during planning
- `segments[]`: one or more planning segments

Optional root fields:

- `chain_scope[]`: preferred chain scope for this planning session
- `session`: planner session metadata (`session_id`, `cursor`)
- `meta`
- `extensions`

Normative rules:

- unknown top-level fields MUST be rejected.
- `pack_snapshot.hash` and `catalog_snapshot.hash` SHOULD be stable content hashes.
- execution stage MUST verify snapshot hashes before compile/run handoff.

---

## 3. Segment shape

Each `segment` represents one incremental planning step.

Required segment fields:

- `segment_id`: stable segment id in current planning session
- `cursor_in`: cursor consumed by this segment
- `cursor_out`: cursor produced by this segment
- `done`: whether intent is fully planned after this segment
- `steps[]`: step list for this segment

Optional segment fields:

- `summary`: short human-readable segment summary
- `extensions`

Normative rules:

- unknown segment fields MUST be rejected.
- `steps[]` MUST be non-empty.
- host SHOULD persist `cursor_out` in checkpoint as the next resume cursor.

---

## 4. Step shape (LLM-friendly)

Required step fields:

- `id`: step id unique within the segment
- `kind`: `query|action|assert|branch`
- `candidate_ref`: candidate reference (`protocol@version/id`)
- `inputs`: input object for candidate params

`inputs` semantics:

- `inputs` values SHOULD use ValueRef forms (`lit/ref/cel/object/array`).
- `inputs.*.cel` is allowed for deterministic value computation (for example amount transforms), not only boolean gating.

`kind` semantics (vNext current behavior):

- `query` and `action` are direct candidate kinds.
- `assert` and `branch` are control-intent labels for planner readability; they still require `candidate_ref + inputs`.
- compiler MUST resolve `candidate_ref` kind from discovered candidates and lower control-labeled steps into executable `query_ref` or `action_ref` nodes.
- compiler MUST preserve original control label in plan node trace metadata (`extensions.plan_sketch.step_kind`) for auditability.

Optional step fields:

- `depends_on[]`: step ids this step depends on
- `stores`: map of `candidate_return_field -> slot_name`
- `when`: condition object
- `until`: post-check ValueRef
- `retry`: retry policy object
- `timeout_ms`: positive integer timeout budget for retry lifecycle
- `constraint_templates[]`: named safety templates with parameters
- `reason`
- `extensions`

`stores` semantics:

- compiler maps candidate return fields into named slots
- later steps can reference slots in a compiler-defined syntax
- planner SHOULD store only fields needed by downstream steps

`when` semantics:

- `when.cel` is an optional CEL gate expression for this step
- CEL context and allowed built-ins are defined by policy/expressions specs
- CEL expressions MUST remain deterministic and side-effect free.

Runtime control semantics:

- `until` is evaluated after step execution using ValueRef semantics.
- `retry` controls re-execution when `until` is not met:
  - `interval_ms`: required positive integer
  - `max_attempts`: optional positive integer
  - `backoff`: currently `fixed`
- `timeout_ms` is an optional positive integer that caps total retry lifecycle.

`when` vs `until`:

- `when` gates whether the step is attempted.
- `until` checks whether the attempted step has reached completion criteria.

`constraint_templates` semantics:

- references template names declared by active policy/pack
- template contract and CEL scope are defined in `specs/ais-2-constraint-templates.md`

Normative rules:

- unknown step fields MUST be rejected.
- `candidate_ref` MUST resolve to a candidate in the same snapshot.
- for `kind=assert|branch`, unresolved candidate kind MUST be rejected with compile issue.
- compiler MUST reject unresolved `depends_on` ids.
- compiler MUST reject any step that resolves to a candidate/execution type not allowed by active pack.
- compiler MUST validate `until/retry/timeout_ms` field shapes before execution handoff.

---

## 5. Segmented planning handoff

Recommended host loop:

1) LLM proposes a `PlanSketch` segment  
2) host validates schema  
3) host compiles segment to `ais-plan/0.0.3` nodes  
4) host runs engine  
5) host feeds structured status/errors back with updated cursor

On compile/validation failure, host SHOULD return structured issues and request a revised segment instead of rewriting all segments.

---

## 6. Authority schema

- JSON Schema: `schemas/0.0.2/plan-sketch.schema.json`
