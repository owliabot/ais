# AIS-2I: Segmented Agent Planning Tools (`ais-agent-planning-tools/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

This document defines the vNext planner tool-calling contract for segmented planning.

It complements:

- Intent input: `specs/ais-2-agent-intent.md`
- Plan sketch output: `specs/ais-2-plan-sketch.md`
- Executable candidates: `specs/ais-1-executable-candidates.md`
- Constraint templates and CEL scope: `specs/ais-2-constraint-templates.md`
- Engine command channel: `specs/ais-2-engine-commands.md`

---

## 1. Purpose and model

The planner MUST NOT generate executable plan nodes directly in vNext mode.

Planner flow is segmented:

1) begin planning session  
2) propose one segment  
3) host compiles/runs segment and returns state or error  
4) planner proposes next segment or revises current segment

The planning contract uses three tool calls:

- `plan.begin`
- `plan.propose_segment`
- `plan.revise_segment`

---

## 2. Shared terms

- `session_id`: planning session identifier produced by `plan.begin`
- `snapshot_hash`: identity hash of planning boundary (pack + catalog + chain scope + policy mode)
- `cursor`: opaque host-issued cursor for incremental planning state
- `segment`: one `PlanSketch` segment payload
- `issues`: structured machine-readable problem list

Normative rules:

- planner MUST treat `cursor` as opaque.
- host MUST reject tool calls whose `snapshot_hash` does not match active session boundary.
- planner MUST NOT assume session continuity across different `snapshot_hash`.

### 2.1 Canonical multi-chain planning context

When host provides `state_summary`, it SHOULD include chain-agnostic canonical slots under:

- `state_summary.canonical_context.chain_refs[]`
- `state_summary.canonical_context.account_refs[]`
- `state_summary.canonical_context.asset_refs[]`
- `state_summary.canonical_context.amount_refs[]`
- `state_summary.input_registry` (canonical `inputs.*` registry for planner refs)

Each item SHOULD include at least:

- `id`: slot identifier
- `ref`: canonical runtime path (normally `inputs.*`)

Field-specific value keys:

- chain item: `chain_ref`
- account item: `account_ref`
- asset item: `address` (optional: `chain_ref/symbol/decimals`)
- amount item: `amount_ref` (or normalized `amount_human/amount_atomic`)

Normative rules:

- planner SHOULD prefer canonical context slots over chain-specific naming guesses.
- planner SHOULD use `state_summary.input_registry.known_refs` as the only source for `inputs.*` refs.
- host MAY keep backward-compatible fields in parallel, but canonical context is the preferred contract for multi-chain planning.

---

## 3. Tool: `plan.begin`

Purpose:

- initialize one segmented planning session and freeze boundary snapshot

Input:

- `intent`: `ais-agent-intent/0.0.1`
- `pack_snapshot_hash`: active pack hash
- `catalog_hash`: candidate/catalog hash
- `chain_scope[]?`: optional planning scope override

Output:

- `schema`: `ais-agent-planning-tools/0.1.0`
- `tool`: `plan.begin`
- `session_id`
- `snapshot_hash`
- `cursor`: initial cursor (for first segment)
- `limits`:
  - `max_rounds`
  - `max_segments`

Normative rules:

- host MUST return deterministic `snapshot_hash` for the same boundary inputs.
- host MUST reject empty intent.
- planner MUST use returned `session_id/snapshot_hash/cursor` for subsequent calls.

---

## 4. Tool: `plan.propose_segment`

Purpose:

- propose next segment from current cursor and latest runtime summary

Input:

- `session_id`
- `snapshot_hash`
- `cursor`
- `state_summary?` (structured summary from host, including previous segment outcomes)

Output:

- `schema`: `ais-agent-planning-tools/0.1.0`
- `tool`: `plan.propose_segment`
- `status`: `proposed|unavailable|invalid`
- `cursor_next?`
- `done`: boolean
- `segment?`: `PlanSketch` segment object
- `issues?`: `Issue[]`
- `error?`: `{ reason_code, message?, details? }`

Normative rules:

- when `status=proposed`:
  - `segment` MUST be present
  - `cursor_next` MUST be present
- when `status=unavailable|invalid`:
  - `error.reason_code` MUST be present
- if `done=true`, planner MUST NOT propose additional segments in this session.

---

## 5. Tool: `plan.revise_segment`

Purpose:

- revise segment proposal after compile/runtime failure

Input:

- `session_id`
- `snapshot_hash`
- `cursor`
- `previous_error`:
  - `reason_code`
  - `node_id?`
  - `details?`
  - `issues?` (schema/compile/policy issues)
- `last_segment?`

Output:

- same envelope and status model as `plan.propose_segment`

Normative rules:

- planner MUST prefer minimal mutation to unresolved/future work.
- planner MUST NOT mutate already-completed segment history.
- host MUST enforce this rule during compile/handoff even if planner violates it.

---

## 6. `issues` minimum shape

Each issue object:

- `kind`: `schema_error|compile_error|policy_error|runtime_error`
- `reason_code`: stable code
- `field_path?`: pointer/path to problematic field
- `message`: short text
- `suggestion?`: optional machine-friendly fix hint

For full issue contract and normalization rules, see `specs/ais-2-compiler-issues.md`.

Reason codes (minimum common set):

- `candidate_not_found`
- `candidate_chain_not_allowed`
- `execution_type_not_allowed`
- `missing_required_input`
- `input_type_mismatch`
- `constraint_violation`
- `policy_requires_confirm`
- `segment_mutation_not_allowed`

---

## 7. Round termination policy

Host MUST implement bounded planning:

- stop when `done=true`
- stop when `status=unavailable|invalid`
- stop when `max_rounds` reached
- SHOULD stop early on repeated `invalid` with same `reason_code`

On stop due to limits:

- host SHOULD emit structured planner error with `reason_code=planner_round_limit_reached`.

---

## 8. Candidate tool interplay

Planner may use candidate tools between segment calls:

- `list_candidates` for compact index snapshot
- `get_candidate_detail` for selected refs only

Normative guidance:

- planner SHOULD fetch detail lazily and minimally.
- host SHOULD enforce ref count limits for detail fetch to control token usage.

---

## 9. Execution handoff

When `status=proposed`, host handoff SHOULD be:

1) validate segmented tool output shape  
2) validate segment against `ais-plan-sketch/0.1.0` subset rules  
3) compile segment to executable plan nodes  
4) run engine and checkpoint state  
5) return summarized state/errors for next segment

Checkpoint persistence/resume invariants are defined in `specs/ais-2-agent-checkpoint.md`.

---

## 10. Authority schema

- JSON Schema: `schemas/0.0.2/agent-planning-tools.schema.json`
- Compiler issues schema: `schemas/0.0.2/compiler-issues.schema.json`
