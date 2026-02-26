# AIS-2G: Agent Intent Contract

Status: Draft  
Spec Version: 0.0.2

This document defines the normalized contract for `agent intent` input.

For segmented planner tool-calling in vNext, see `specs/ais-2-agent-planning.md`.

---

## 1. Intent input (`ais-agent-intent/0.0.1`)

Purpose:

- carry a natural-language user goal into the runner/planner loop
- attach optional execution constraints and target chains

Required fields:

- `schema`: MUST be `ais-agent-intent/0.0.1`
- `intent`: non-empty natural-language instruction

Optional fields:

- `target_chains[]`: preferred chain ids (e.g. `eip155:1`)
- `constraints`:
  - `approvals_mode`: `safe|assist|yolo`
  - `must_confirm`: force manual confirmation for transfer/write actions
  - `max_planner_rounds`: upper bound for planner retries
  - `max_execution_rounds`: upper bound for run/repair loop
- `context`: extra structured hints for intent parsing

Normative rules:

- unknown top-level fields MUST be rejected.
- `intent` MUST NOT be empty after trim.
- when `constraints.must_confirm=true`, runtime policy MUST NOT auto-bypass `need_user_confirm`.

Mode mapping notes:

- `constraints.approvals_mode` (if provided) is a host override hint for pack approval mode in this run.
- if omitted, host SHOULD use active pack approvals mode.
- `must_confirm=true` has higher priority than `approvals_mode=assist|yolo` for transfer/write actions.

Authority schema:

- `schemas/0.0.2/agent-intent.schema.json`

---

## 2. Planner coupling in vNext

In vNext, intent planning is tool-calling + segmented only:

- `plan.begin`
- `plan.propose_segment`
- `plan.revise_segment`

The planner output contract in vNext is segmented tools only.

Normative rules:

- host MUST NOT treat a raw LLM `ais-plan` object as executable handoff.
- host MUST compile validated `PlanSketch` segments into executable `ais-plan/0.0.3`.
- host MUST perform policy/allowlist checks before execution.

Authority specs:

- `specs/ais-2-agent-planning.md`
- `specs/ais-2-plan-sketch.md`
