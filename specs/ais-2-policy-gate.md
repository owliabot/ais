# AIS-2G: Policy Gate I/O — v0.0.1

Status: Draft  
Spec Version: 0.0.2

This document standardizes the **policy gate** contract used by engines/runners to:

- extract a deterministic, auditable “risk snapshot” for a node about to execute, and
- produce a standardized decision:
  - `ok`
  - `need_user_confirm`
  - `hard_block`

Policy gate outputs are designed to be surfaced as engine events (see `specs/ais-2-engine-events.md`) and to be used by agent loops and UIs consistently.

---

## 1. Terminology

- **Node**: an execution plan node (`ais-plan/0.0.3`) that may cause side effects (e.g. tx broadcast) or require policy checks.
- **Policy gate**: a pure decision layer that runs *before execution* (and MAY run again after solver patches).
- **Pack**: the active policy boundary (`ais-pack/0.0.2`) that provides allowlists and thresholds.

---

## 2. Core types

### 2.1 `PolicyGateInput` (snapshot)

`PolicyGateInput` is a structured snapshot extracted from:

- the node identity (`node_id`, `chain`, `execution_type`, `action_ref`)
- action metadata (`risk_level`, `risk_tags`)
- resolved params/preview fields (amounts, slippage, approval fields)

Field semantics:

- `chain`: CAIP-2 string.
- `action_ref`: SHOULD be the fully-qualified reference `protocol@version/actionId`.
- Amounts:
  - `spend_amount`, `approval_amount` MUST be **base-unit integer strings** (no floats).
- `slippage_bps`: integer basis points.
- Missingness classification:
  - `hard_block_fields[]`: missing required fields that make the action unsafe to classify (e.g. missing chain).
  - `missing_fields[]`: required-under-this-action fields that are absent (e.g. swap missing slippage).
  - `unknown_fields[]`: fields that are optional but materially affect risk (e.g. `unlimited_approval` not provided).

### 2.2 `PolicyGateOutput` (decision)

Policy gate output is one of:

- `ok`
- `need_user_confirm`
- `hard_block`

All outputs carry:

- `reason`: short, human-readable primary reason (stable phrasing recommended)
- `details`: machine-readable evidence map

`details` SHOULD include:

- `hit_reasons[]`: list of normalized rule hits (stable ids recommended)
- `thresholds`: relevant policy thresholds (summarized)
- `violations[]`: structured violations (optional)
- `matched_constraints[]`: matched CEL constraint ids (when CEL constraints are enabled)

### 2.3 `ConfirmationSummary` and `confirmation_hash`

When a policy gate output is `need_user_confirm`, engines/runners MUST provide a stable confirmation identifier to:

- bind confirmation to semantics (avoid TOCTOU),
- support dedupe / replay / audit.

`ConfirmationSummary` is a structured, stable subset of:

- the gate input (identity + risk)
- the gate output (reason + selected details)

`confirmation_hash` is derived from `ConfirmationSummary` via a stable hash (see §5).

---

## 3. Mapping to engine events (normative)

When a node is blocked by policy gate, the engine MUST emit:

- `need_user_confirm`, or
- `hard_block`

For `need_user_confirm`, `event.data.details` MUST include at least:

- `node_id`
- `action_ref`
- `hit_reasons: string[]`
- `confirmation_summary`
- `confirmation_hash`

See `schemas/0.0.2/engine-event.schema.json` for the authority event schema constraints.

---

## 4. Minimal required fields

### 4.1 `PolicyGateInput` minimal set

Engines MUST produce a `PolicyGateInput` with at least:

- `chain`
- `execution_type` (if known)
- `action_ref` (if action identity is known)
- `risk_level` (if action provides it)

If any of the following is missing, engines SHOULD add it to `hard_block_fields`:

- `chain`

### 4.2 Hard block vs confirm (missingness)

Normative decision behavior (missingness-only):

Policy gate MUST decide as follows before applying allowlist/threshold checks:

1) If `hard_block_fields` is non-empty: output `hard_block`.
2) Else if `missing_fields` is non-empty:
   - output `hard_block` if `hard_block_on_missing = true`, otherwise
   - output `need_user_confirm`.
3) Else if `unknown_fields` is non-empty: output `need_user_confirm`.
4) Else: continue with allowlist/threshold checks.

Note:
- `hard_block_on_missing` is an engine/host enforcement option (default `false`).
- The missingness semantics (`hard_block_fields` vs `missing_fields` vs `unknown_fields`) MUST remain stable across implementations.

---

## 5. Stable confirmation hash (normative)

This spec standardizes `confirmation_hash` as:

- `sha256` of a stable JSON encoding of `ConfirmationSummary`
- ignoring timestamp-like keys:
  - `ts`, `timestamp`, `created_at`, `updated_at`

Stable JSON encoding requirements:

- sort object keys lexicographically
- preserve array order
- output compact JSON (no insignificant whitespace)

The expected output format is:

- lowercase hex string (no `0x` prefix)

Rationale:
- the confirmation hash must be portable across implementations and stable across retries.

---

## 6. Authority Schemas

This repository defines authority JSON Schemas for:

- Engine events: `schemas/0.0.2/engine-event.schema.json`
- (Optional, recommended) policy gate types:
  - `schemas/0.0.2/policy-gate-input.schema.json`
  - `schemas/0.0.2/policy-gate-output.schema.json`
  - `schemas/0.0.2/confirmation-summary.schema.json`

---

## 7. CEL constraints execution model (normative)

For vNext, policy gate SHOULD use constraint templates as canonical input.
Template contract and CEL scope are defined in `specs/ais-2-constraint-templates.md`.

When pack policy defines raw `policy.constraints[]`, policy gate MUST evaluate each CEL expression against `input` (the normalized `PolicyGateInput` object).

When template references are present on planning steps, host/compiler MUST resolve templates into effective constraints before gate evaluation.

Required behavior:

- evaluation order is list order from pack.
- each match contributes one `matched_constraints[]` entry using constraint `id`.
- decision merge:
  - any matched `hard_block` constraint => output `hard_block`
  - else if any matched `need_user_confirm` constraint => output `need_user_confirm`
  - else continue with remaining checks.

Error handling:

- invalid CEL expression SHOULD produce `hard_block` with stable reason code (recommended: `policy_constraint_eval_error`) unless host explicitly opts into permissive mode.

CEL scope constraints (normative):

- allowed roots: `input`, `params`, `policy`
- forbidden roots: `nodes`, `runtime`, `env`, or any IO/network handle
- CEL evaluation MUST be deterministic and side-effect-free.

Slim profile rule:

- AIS 0.0.2 slim profile does not define a fixed-threshold path.

---

## 8. Intent-mode confirmation semantics (normative)

When execution is initiated from `ais-agent-intent/0.0.1`, policy gate behavior MUST remain explicit and conservative for transfer/write actions.

Default behavior:

- `safe`: transfer/write actions MUST pause as `need_user_confirm` unless policy gate outputs `hard_block`.
- `assist`: transfer/write actions MAY be auto-approved only when `risk_level <= llm_may_approve_max_risk_level`; otherwise MUST pause as `need_user_confirm`.
- `yolo`: host MAY auto-approve `need_user_confirm`, but MUST NOT bypass `hard_block`.

Required reason_code alignment:

- `need_user_confirm` in intent mode SHOULD use stable `reason_code=intent_need_user_confirm`.
- LLM auto-approval in assist mode SHOULD record `reason_code=intent_assist_auto_approved`.
- LLM/host auto-approval in yolo mode SHOULD record `reason_code=intent_yolo_auto_approved`.
- if `constraints.must_confirm=true` is active on intent input, any write action requiring confirmation MUST use `reason_code=intent_must_confirm`.

Normative safety rules:

- `constraints.must_confirm=true` MUST force manual confirmation for transfer/write actions even in `assist|yolo`.
- no intent mode may bypass:
  - `hard_block`,
  - pack allowlists,
  - registered execution handlers.
