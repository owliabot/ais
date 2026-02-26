# AIS-4: Conformance — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document will define conformance test vectors for:

- ValueRef evaluation
- Numeric conversions (`to_atomic` / `to_human` / `mul_div`)
- EVM ABI encoding (incl. tuples)
- Chain pattern matching
- Registry `specHash` canonicalization
- Engine command schema validation
- Pack approval decision semantics (risk_level → confirmation)
- Execution handler registration + plugin allowlist decision semantics
- Protocol install governance decision semantics

## 1. Conformance vectors directory

This repository contains conformance vectors under:

- `specs/conformance/vectors/*.json`
- core/extended profile manifest: `specs/conformance/profiles/core-files.json`
- extended manifest: `specs/conformance/profiles/extended-files.json`

Vectors are intended to be:
- **Deterministic** (no timestamps, no network IO)
- **Portable** across implementations (SDKs in other languages can reuse the same vectors)
- **Minimal but load-bearing**: cover semantics that tend to regress during refactors

### 1.1 Core vs extended profiles (normative)

Conformance vectors are split into two execution profiles:

- `core`: blocking set for CI/PR gating (security + execution correctness critical path)
- `extended`: non-blocking set for broader compatibility/regression checks

Vector files SHOULD set top-level `profile`:

- `"profile": "core" | "extended" | "mixed"`

Default execution policy (recommended):

- PR/CI required checks run `core`.
- nightly/release checks run `core + extended`.

Reference execution (TS SDK conformance runner):

- core (default): `AIS_CONFORMANCE_PROFILE=core npm --prefix ts-sdk test -- tests/conformance-vectors.test.ts`
- extended: `AIS_CONFORMANCE_PROFILE=extended npm --prefix ts-sdk test -- tests/conformance-vectors.test.ts`
- all: `AIS_CONFORMANCE_PROFILE=all npm --prefix ts-sdk test -- tests/conformance-vectors.test.ts`

Current core semantic set:

- engine command schema (`engine-command.json`)
- pack approvals (`pack-approvals.json`, `pack-approvals-decision.json`)
- policy-gate missingness (`policy-gate-missingness.json`)
- execution handler registration (`plugin-type-registration.json`)
- protocol install governance (`protocol-install-decision.json`)

Command vectors:
- Schema validation for `ais-engine-command/0.0.1` is provided as a portable conformance kind.

### 2.2 JSON Schema validation vectors (`json_schema_validate`)

This repository defines a portable conformance kind:

- `kind = "json_schema_validate"`

Input fields:
- `schema_id` (string): the AIS document schema discriminator (e.g. `ais-engine-command/0.0.1`)
- `value` (any): the JSON value to validate against the authority JSON Schema for `schema_id`

Expected fields:
- `valid` (boolean): whether validation must pass

Engines/SDKs that claim conformance for a given document type SHOULD pass all corresponding `json_schema_validate` cases.

### 2.3 Pack approvals decision vectors (`pack_approvals_decision`)

This repository defines a portable conformance kind:

- `kind = "pack_approvals_decision"`

It tests the normative approval decision algorithm defined in `specs/ais-1-pack.md`:

- inputs: `approvals` config + action `risk_level`
- outputs: whether confirmation is needed and who is permitted to approve

### 2.4 Confirmation hash vectors (`confirmation_hash`)

This repository defines a portable conformance kind:

- `kind = "confirmation_hash"`

It tests the normative confirmation hash algorithm defined by the policy gate spec (`specs/ais-2-policy-gate.md`):

- input: a `ConfirmationSummary`-like object (free-form JSON)
- expect: `hash_sha256_hex`

Engines/SDKs SHOULD implement this hash as:

- stable JSON encoding (sorted keys, compact),
- remove keys `ts`, `timestamp`, `created_at`, `updated_at` (at any object level), then
- `sha256` over the resulting JSON bytes, hex-encoded lowercase without prefix.

### 2.5 Policy gate missingness decision vectors (`policy_gate_missingness_decision`)

This repository defines a portable conformance kind:

- `kind = "policy_gate_missingness_decision"`

It tests the **normative missingness-only decision** defined in `specs/ais-2-policy-gate.md` (§4.2):

- input: `hard_block_fields[]`, `missing_fields[]`, `unknown_fields[]`, and optional `hard_block_on_missing`
- expect: output `kind` (`ok|need_user_confirm|hard_block`)

### 2.6 Execution handler registration decision vectors (`execution_handler_registration_decision`)

This repository defines a portable conformance kind:

- `kind = "execution_handler_registration_decision"`

It tests the normative routing rules from AIS capabilities/plan specs:

- input: `execution_type`, `core_execution_types[]`, `registered_execution_types[]`, and optional `pack_plugin_allowlist`
- expect: output `decision` (`allow|reject`) and optional stable `reason_code`

Normative checks covered by vectors:

- unregistered `execution.type` MUST be rejected (core and plugin types),
- for plugin types, pack allowlisting is necessary but not sufficient,
- when pack plugin allowlist is active, plugin types not in allowlist MUST be rejected.

### 2.7 `replace_plan` behavior testing boundary

Portable conformance in this repository intentionally covers `replace_plan` at the **schema contract** level (via `json_schema_validate` vectors in `engine-command.json`).

Forbidden-mutation behavior is intentionally treated as **implementation-level integration testing**, because it depends on runtime state/history (completed nodes, checkpoint lineage, emitted events) rather than static payload validity.

Implementations SHOULD provide fixture-based tests for:

- mutating/deleting completed nodes (default reject),
- replacing plan with high-risk structural diffs (confirm or reject per policy),
- successful replace with auditable `plan_replaced` event hashes.

### 2.8 Protocol install governance vectors (`protocol_install_decision`)

This repository defines a portable conformance kind:

- `kind = "protocol_install_decision"`

It tests normative policy decisions for dynamic protocol install:

- input:
  - `mode` (`safe|assist|yolo`)
  - `source_kind` (`local_path|registry_ref|remote_url|llm_generated`)
  - policy toggles (`allowed_sources`, `require_signature`)
  - optional source metadata (`has_signature`)
- expect:
  - `decision` (`allow|need_user_confirm|reject`)
  - optional stable `reason_code`

Normative checks covered by vectors:

- `safe` rejects unsafe dynamic sources by default,
- `assist` requires confirmation for high-risk/dynamic sources,
- `yolo` can allow broader sources but cannot bypass signature/integrity gates when configured.

## 2. Vector file shape (non-normative, but recommended)

Each vector file SHOULD be a JSON object:

```json
{
  "schema": "ais-conformance/0.0.2",
  "cases": [
    { "id": "example", "kind": "evm_json_abi_encode", "input": { }, "expect": { } }
  ]
}
```

Common fields:
- `id` (string): stable identifier for referencing the case
- `kind` (string): case type (e.g. `cel_eval`, `evm_json_abi_encode`, `select_execution_spec`, `workflow_plan`)
- `input`: structured input data
- `expect`: structured expected output, or `error_contains` for negative tests

### 2.1 Numeric vectors (normative)

Implementations that claim conformance for AIS numeric model MUST pass all cases in:

- `specs/conformance/vectors/numeric.json`

These vectors cover:

- `to_atomic()` exactness + truncation disallowed
- `to_human()` canonical formatting
- `mul_div()` integer semantics + error conditions

## 3. Canonicalization (JCS)

For `specHash`, vectors assume RFC 8785 JCS-style canonical JSON serialization:
- sort object keys lexicographically
- preserve array order
- serialize using JSON string escaping rules

The hash algorithm is implementation-defined by the engine/registry. Vectors MAY provide a keccak256 example.
