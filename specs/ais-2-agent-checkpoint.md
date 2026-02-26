# AIS-2K: Agent Checkpoint Contract (`ais-agent-checkpoint/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

This document defines a durable checkpoint contract for segmented planning/execution.
Its goals are:

- crash-safe resume
- side-effect idempotency (no duplicate spend)
- stable audit chain across plan epochs

It complements:

- Segmented planning tools: `specs/ais-2-agent-planning.md`
- Plan contract: `specs/ais-2-plan.md`
- Engine events: `specs/ais-2-engine-events.md`
- Engine commands: `specs/ais-2-engine-commands.md`

---

## 1. Root shape (`ais-agent-checkpoint/0.1.0`)

Required fields:

- `schema`: MUST be `ais-agent-checkpoint/0.1.0`
- `run_id`: stable run identifier
- `session_id`: segmented planning session id
- `checkpoint_seq`: monotonic checkpoint sequence
- `saved_at`: RFC3339 timestamp
- `plan_epoch`: active plan epoch
- `active_plan_hash`: hash of currently active plan
- `plan_hash_history[]`: non-empty list, last item MUST equal `active_plan_hash`
- `pack_snapshot_hash`: active pack snapshot hash
- `catalog_hash`: active candidate/catalog hash
- `cursor`: latest planning cursor
- `completed_node_ids[]`: completed node ids in active/previous epochs
- `approvals_ledger[]`: confirmation decisions ledger
- `side_effects[]`: emitted side effects ledger

Optional fields:

- `runtime`: runtime state snapshot needed for deterministic resume
- `paused_reason`
- `plan_snapshot`: optional serialized active plan for hash mismatch recovery
- `extensions`

Normative rules:

- unknown top-level fields MUST be rejected.
- `checkpoint_seq` MUST be strictly monotonic within one `run_id`.
- `plan_epoch` MUST increase only when plan replacement is accepted.
- checkpoint writer MUST flush atomically (write temp + fsync + rename).

---

## 2. Plan epoch and mutation invariants

When `replace_plan` is accepted:

- checkpoint MUST append new hash to `plan_hash_history`.
- checkpoint MUST increment `plan_epoch`.
- checkpoint MUST preserve completed-node history from previous epochs.

Forbidden mutations on resume (MUST reject):

- deleting/rewriting nodes already recorded in `completed_node_ids`.
- reusing a completed node id with different semantics.

On hash mismatch between runtime input plan and checkpoint:

- host MUST prefer checkpoint `plan_snapshot` if present and valid.
- if snapshot is absent/invalid, host MUST hard block resume.

---

## 3. Approval ledger invariants

Each `approvals_ledger` item:

- `confirmation_hash`
- `node_id`
- `decision`: `approve|deny`
- `recorded_at`
- `reason_code?`

Normative rules:

- for the same `confirmation_hash`, decisions MUST be immutable.
- duplicate confirmation commands with same decision MUST be no-op.
- conflicting decision for same hash MUST be rejected.

Resume behavior:

- if run is paused on `need_user_confirm` and ledger already contains matching `confirmation_hash`, host SHOULD auto-apply the same decision deterministically.

---

## 4. Side-effect idempotency invariants

Each `side_effects` item MUST conform to `ais-side-effect-record/0.1.0`.

Minimum fields (from side-effect contract):

- `effect_type` (for example `tx`)
- `idempotency_key`
- `node_id`
- `chain`
- `execution_type`
- `status`: `prepared|sent|confirmed|reverted|unknown`
- `observed_at`

Recommended fields:

- `tx_hash`
- `nonce`
- `provider_ref`
- `reason_code`
- `details` (non-secret adapter metadata)

Normative rules:

- `idempotency_key` MUST be stable for the same logical action.
- on resume, before executing a write node, host MUST check `side_effects`:
  - if existing entry is `sent|confirmed`, host MUST NOT re-send blindly
  - host MUST reconcile with chain state first (for example receipt lookup)
- if receipt confirms success, node MUST be marked completed without re-execution.

---

## 5. Snapshot binding invariants

Checkpoint is bound to planning boundary:

- `pack_snapshot_hash`
- `catalog_hash`
- optional chain scope from runtime/session

Resume with boundary mismatch:

- default behavior MUST be hard block (`snapshot_mismatch`).
- host MAY support explicit migration flow, but MUST require explicit user confirmation and create a new checkpoint lineage.

---

## 6. Recommended resume algorithm

1) load checkpoint and validate schema  
2) verify snapshot binding (`pack_snapshot_hash`, `catalog_hash`)  
3) reconstruct active plan by `active_plan_hash` / `plan_snapshot`  
4) restore completed nodes and command dedupe state  
5) reconcile side effects against chain/external status  
6) resume from paused point or continue next schedulable node

Implementations SHOULD emit structured pause/error reason codes:

- `checkpoint_invalid`
- `checkpoint_plan_hash_mismatch`
- `snapshot_mismatch`
- `side_effect_reconcile_reverted`

---

## 7. Authority schema

- JSON Schema: `schemas/0.0.2/agent-checkpoint.schema.json`
- Side-effect Schema: `schemas/0.0.2/side-effect-record.schema.json`
