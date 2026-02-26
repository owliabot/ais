# AIS-2S: Side-Effect Record Contract (`ais-side-effect-record/0.1.0`)

Status: Draft  
Spec Version: 0.0.2

This document defines a unified side-effect record used by:

- engine event stream (event data payload)
- checkpoint persistence and resume idempotency
- future execution-type adapters

The goal is to eliminate chain-specific field guessing in runner and provide one stable contract.

---

## 1. Canonical shape

Required fields:

- `schema`: MUST be `ais-side-effect-record/0.1.0`
- `effect_type`: logical side-effect class (for example `tx`, `approval`, `intent_lock`)
- `idempotency_key`: stable key for the same logical action
- `node_id`: originating plan node id
- `chain`: chain scope (for example `eip155:1`, `solana:mainnet`)
- `execution_type`: execution adapter identifier (for example `evm_call`, `solana_instruction`)
- `status`: `prepared|sent|confirmed|reverted|unknown`
- `observed_at`: RFC3339 timestamp when this record is observed

Optional fields:

- `tx_hash`
- `nonce`
- `provider_ref`
- `reason_code`
- `details` (object, adapter-specific non-secret metadata)

Normative rules:

- unknown top-level fields MUST be rejected.
- `idempotency_key` MUST remain stable for retries/restarts of the same logical action.
- `status` transitions SHOULD be monotonic (`prepared -> sent -> confirmed|reverted`); fallback `unknown` is allowed for partial observations.
- secrets and signing material MUST NOT appear in `details`.

---

## 2. Transport and embedding points

### 2.1 Engine event stream

When engine observes a side-effect, it SHOULD emit:

- `event.type = side_effect_observed`
- `event.data.record = SideEffectRecord`

This provides event-driven ledger rebuild and replay consistency.

### 2.2 Checkpoint persistence

Checkpoint `side_effects[]` entries MUST use this exact record contract.

Resume/reconcile logic MUST operate on `idempotency_key + status`, not on chain-specific output guesswork.

---

## 3. Idempotency and safety semantics

Before re-executing value-moving nodes, host MUST inspect side-effect records:

- existing `status in {sent, confirmed}` for same `idempotency_key`:
  - MUST reconcile external state first
  - MUST NOT blindly resend
- existing `reverted`:
  - MAY retry under policy and adapter strategy
- existing `unknown`:
  - SHOULD query external source/provider before deciding

---

## 4. Authority schema

- JSON Schema: `schemas/0.0.2/side-effect-record.schema.json`
