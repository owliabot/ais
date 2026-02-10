# AIS-4: Conformance — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document will define conformance test vectors for:

- ValueRef evaluation
- Numeric conversions (`to_atomic` / `to_human` / `mul_div`)
- EVM ABI encoding (incl. tuples)
- Chain pattern matching
- Registry `specHash` canonicalization

## 1. Conformance vectors directory

This repository contains conformance vectors under:

- `specs/conformance/vectors/*.json`

Vectors are intended to be:
- **Deterministic** (no timestamps, no network IO)
- **Portable** across implementations (SDKs in other languages can reuse the same vectors)
- **Minimal but load-bearing**: cover semantics that tend to regress during refactors

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
