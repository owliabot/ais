# AIS-2F: Engine Commands — v0.0.1

Status: Draft  
Spec Version: 0.0.2  
Schema: `ais-engine-command/0.0.1`

This document defines the command protocol used to drive an AIS engine in a multi-turn agent/runner loop.

Commands are intended to be:

- **machine-generated** (by an agent, UI, or automation)
- **idempotent** (safe to resend via command `id` dedupe)
- **deterministic** (no hidden side effects beyond what the engine explicitly applies)

The engine command protocol complements:

- Engine events: `specs/ais-2-engine-events.md` (`ais-engine-event/0.0.3`)
- Execution plan: `specs/ais-2-plan.md` (`ais-plan/0.0.3`)
- Intent + segmented planning contracts:
  - `specs/ais-2-agent-intent.md` (`ais-agent-intent/0.0.1`)
  - `specs/ais-2-agent-planning.md` (`ais-agent-planning-tools/0.1.0`)

---

## 1. Transport: JSONL

Commands are typically transported as JSON Lines (JSONL):

- One command envelope per line
- No trailing commas
- Empty lines SHOULD be ignored by runners

Runners MAY accept commands via stdin (recommended for agent-loop integration).

---

## 2. Envelope shape

Each command line MUST be a JSON object:

```json
{
  "schema": "ais-engine-command/0.0.1",
  "command": {
    "id": "cmd-001",
    "type": "apply_patches",
    "data": { }
  }
}
```

Fields:

- `schema` (required): MUST equal `ais-engine-command/0.0.1`
- `command` (required): command payload

The command payload:

- `id` (required): stable identifier used for dedupe / idempotency
- `type` (required): command type string
- `data` (optional): object; defaults to `{}` when omitted

Strictness (normative):

- Unknown fields MUST be rejected (both at envelope level and inside `command`), to avoid silent “best-effort” behavior.

---

## 3. Dedupe / idempotency

Engines MUST treat `command.id` as an idempotency key:

- If a command with the same `id` has already been applied in the current run, a duplicate MUST NOT be applied again.
- Engines MUST surface duplicates as either:
  - accepted-noop, or
  - rejected
  depending on engine configuration.

Rationale:

- Agent loops may retry sending commands on transient IO errors.
- UI integrations may resend commands after reconnecting.

---

## 4. Command types (minimum set)

AIS 0.0.2 standardizes the following minimum set.

### 4.1 `apply_patches`

Applies one or more runtime patches under the engine’s runtime patch guard policy.

```json
{
  "id": "cmd-patch-1",
  "type": "apply_patches",
  "data": {
    "patches": [
      { "op": "set", "path": "inputs.amount_in", "value": "10" }
    ]
  }
}
```

Normative rules:

- Engines MUST validate the patch list and reject invalid patches.
- Engines MUST enforce a runtime patch guard policy (safe defaults MUST NOT allow arbitrary writes to `nodes.*`).
- Engines SHOULD emit patch audit events (e.g. `patch_applied` / `patch_rejected`) with stable audit hashes.

### 4.2 `user_confirm`

Records a user confirmation decision for a specific node (or other confirmation scope defined by the engine).

```json
{
  "id": "cmd-confirm-1",
  "type": "user_confirm",
  "data": { "node_id": "n1", "decision": "approve" }
}
```

Normative rules:

- `decision` MUST be one of: `approve`, `deny`.
- Engines MUST NOT treat missing `node_id` as a global approval.
- If the node was paused for `need_user_confirm`, an approval SHOULD allow the engine to continue.

### 4.3 `user_input`

将用户任意输入写入运行时输入上下文（默认 `runtime.inputs.<input_id>`）。

```json
{
  "id": "cmd-input-1",
  "type": "user_input",
  "data": {
    "input_id": "owner",
    "value": "0xabc...",
    "target_path": "inputs.owner"
  }
}
```

Normative rules:

- `input_id` MUST be non-empty string.
- `value` MUST be present.
- if `target_path` is provided, it MUST start with `inputs.`.

### 4.4 `user_select`

将“选项选择”结果写入运行时输入上下文（默认 `runtime.inputs.<input_id>`）。

```json
{
  "id": "cmd-select-1",
  "type": "user_select",
  "data": {
    "input_id": "token",
    "selected_index": 1,
    "options": [
      { "label": "USDC", "value": "0xA0b8..." },
      { "label": "USDT", "value": "0xdAC1..." }
    ]
  }
}
```

Normative rules:

- `input_id` MUST be non-empty string.
- `selected_value` MAY be provided directly.
- otherwise `selected_index` (1-based) + `options[]` MUST be provided.
- if `target_path` is provided, it MUST start with `inputs.`.

### 4.5 `cancel`

Requests cancellation of the current run.

```json
{
  "id": "cmd-cancel-1",
  "type": "cancel",
  "data": { "reason": "user requested cancel" }
}
```

Normative rules:

- Engines MUST stop making forward progress once cancel is applied.

### 4.6 `replace_plan`

Replaces the active execution plan during a run.

This command exists to support agent-driven plan mutation: the agent may refine the plan after observing runtime conditions (errors, missing inputs, better routes), but the engine must keep the run auditable and policy-bounded.

```json
{
  "id": "cmd-replace-plan-1",
  "type": "replace_plan",
  "data": {
    "plan": {
      "schema": "ais-plan/0.0.3",
      "nodes": [],
      "meta": {},
      "extensions": {}
    },
    "reason": "add precheck + wait-until after seeing receipt timeout"
  }
}
```

Normative rules:

- Engines MUST validate the new plan before using it.
- Engines MUST NOT allow `replace_plan` to bypass:
  - protocol/pack allowlists,
  - execution plugin allowlists,
  - handler registration requirements, or
  - policy gate requirements for write actions.
- Engines SHOULD preserve auditability by recording:
  - a plan hash for the previous and next plan, and
  - a parent link (plan epoch) either in events, checkpoint, or plan extensions.

Recommended behavior (non-normative):

- Engines/runner SHOULD compute a plan diff and require confirmation for high-risk structural changes.
- Engines/runner SHOULD restrict or explicitly confirm edits that would invalidate the executed history (e.g. deleting or rewriting already-completed nodes).

Recommended change policy (non-normative but strongly suggested):

- Allowed (typical safe mutations):
  - add new nodes that depend on already-completed nodes (prechecks, verification, wait-until polling)
  - change args/condition/until/retry/timeout for nodes that have not started yet
  - change scheduling hints / non-semantic metadata (under `extensions`)
- Requires explicit confirmation (high-risk):
  - removing nodes (even if not started)
  - changing dependencies in a way that reorders writes
  - changing execution types or destinations (e.g. different contract address) for write actions
- Forbidden by default (breaks auditability):
  - rewriting or deleting already-completed nodes
  - changing a node id that has already appeared in emitted events/checkpoints

Recommended diff summary fields (for confirmation UI / agent reasoning):

- `before_plan_hash`, `after_plan_hash`
- `added_node_ids[]`, `removed_node_ids[]`, `changed_node_ids[]`
- `notes` (optional human-readable summary)

---

## 4.7 Segmented planner tool mapping (normative)

In intent mode, planner-facing tools and engine commands are distinct layers:

- planner tools (`plan.begin`, `plan.propose_segment`, `plan.revise_segment`) produce `PlanSketch` segments
- engine command transport applies validated compiled results (`replace_plan`, `cancel`, `user_confirm`)

Rules:

- host MUST NOT execute raw planner output directly.
- host MUST compile segment outputs into executable plan nodes before issuing `replace_plan`.
- planner MUST NOT mutate completed-node history; host enforcement is mandatory via `replace_plan` guards.

Termination recommendations:

- host SHOULD bound planner rounds (`max_planner_rounds`).
- host SHOULD terminate early on repeated invalid/unavailable outputs with stable `reason_code`.

---

## 5. Forward compatibility

- Engines MUST reject unknown `command.type` values by default.
- New command types MUST be introduced via:
  1) a new spec revision, and
  2) a schema update (and conformance coverage where applicable).

---

## 6. Authority schema

- JSON Schema: `schemas/0.0.2/engine-command.schema.json`

---

## 7. Conformance boundary for `replace_plan` (normative)

To keep portable conformance deterministic and implementation-agnostic:

- Schema shape and command envelope validity for `replace_plan` MUST be covered by portable conformance vectors (`json_schema_validate` kind).
- Behavioral policy for forbidden mutations (for example, editing completed nodes or rewriting historical node ids) SHOULD be tested as implementation-level integration fixtures, not portable conformance vectors.

Rationale:

- Forbidden-mutation enforcement depends on engine runtime state (completed nodes, checkpoint history, event log), which is not fully captured by a single static vector input/output pair.

Recommended implementation-fixture contract:

- Given:
  - `before_plan`
  - `after_plan`
  - `completed_node_ids`
  - optional command context (`command_id`, `reason`)
- Expect:
  - decision (`allow|reject|need_user_confirm`)
  - stable reason code (for example `replace_plan_forbidden_completed_node_mutation`)
  - when allowed, emitted `plan_replaced` event contains `before_plan_hash` and `after_plan_hash`.
