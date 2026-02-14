# AIS-1C: Workflow — v0.0.3

Status: Draft  
Spec Version: 0.0.3  
Schema: `ais-flow/0.0.3`

Workflows orchestrate protocol/query/action nodes as a DAG. Dynamic node values use `ValueRef` (`lit/ref/cel/detect/object/array`).

## 1. Strictness and extensions

Workflow documents are strict:
- Unknown fields MUST be rejected.
- Implementation-specific data MUST be placed under `extensions`.

`extensions` MAY appear at:
- workflow root
- node object
- nested typed objects that define an `extensions` field in schema

## 2. Root object

Required fields:
- `schema: "ais-flow/0.0.3"`
- `meta: { name, version, ... }`
- `nodes: WorkflowNode[]`

Optional fields:
- `default_chain: "<caip2>"`
- `imports.protocols[]`
- `requires_pack`
- `inputs`
- `policy`
- `preflight`
- `outputs`
- `extensions`

### 2.1 `imports.protocols[]`

Each entry:
- `protocol: "<protocol>@<semver>"` (required)
- `path: "<relative-or-absolute-path>"` (required)
- `integrity: "<hash>"` (optional)
- `extensions` (optional)

Semantics:
- Declares protocol dependencies for deterministic loading.
- Tooling SHOULD validate that referenced protocol docs exist and match `protocol`.

### 2.2 `requires_pack`

`requires_pack` pins workflow to a pack identity:
- `name` (required)
- `version` (required)

Runner/SDK SHOULD ensure the selected pack includes all protocols used by workflow nodes and respects chain scope rules.

## 3. Node model

Node required fields:
- `id`
- `type` (`query_ref` | `action_ref`)
- `protocol` (`<protocol>@<version>`)

Node optional fields:
- `chain`
- `query` / `action` (by node `type`)
- `args`
- `calculated_overrides`
- `deps`
- `condition`
- `assert`
- `assert_message`
- `until`
- `retry`
- `timeout_ms`
- `extensions`

## 4. Chain resolution

Each executable node MUST resolve to exactly one concrete CAIP-2 chain id.

Resolution order:
1. `nodes[].chain`
2. `workflow.default_chain`

If neither yields a chain, validation/compile MUST fail.

## 5. DAG and scheduling

Workflow nodes form a DAG:
- explicit edges: `deps: ["<node_id>", ...]`
- implicit edges: `ref`/`cel` references to `nodes.<id>...`

Engines MUST reject cycles.

Scheduling recommendations:
- ready nodes MAY run in parallel
- same-account same-chain EVM write broadcasts SHOULD be serialized (nonce safety)

## 6. Node lifecycle semantics

Recommended per-attempt evaluation order:
1. Evaluate `condition` (pre-check); falsy => node skipped
2. Execute node (`query_ref` / `action_ref`)
3. Apply outputs/writes to runtime
4. Evaluate `assert`; falsy => fail-fast
5. Evaluate `until`; falsy => retry loop (subject to retry/timeout)

### 6.1 `condition`

- Evaluated before execution.
- Falsy means skip, not failure.

### 6.2 `assert` / `assert_message`

- Evaluated once after successful execution and runtime writeback.
- Falsy is a hard failure for the node attempt.
- `assert_message` SHOULD be used as user-facing error text when provided.

### 6.3 `until` / `retry` / `timeout_ms`

- `until`: post-check ValueRef.
- `retry.interval_ms`: required positive integer.
- `retry.max_attempts`: optional positive integer.
- `retry.backoff`: currently `fixed`.
- `timeout_ms`: optional positive integer, caps overall polling lifecycle.

## 7. Minimal example

```yaml
schema: "ais-flow/0.0.3"
meta:
  name: "swap-with-guard"
  version: "0.0.1"
default_chain: "eip155:1"
imports:
  protocols:
    - protocol: "uniswap-v3@0.0.2"
      path: "./protocols/uniswap-v3.ais.yaml"
requires_pack:
  name: "safe-defi"
  version: "0.0.2"
inputs:
  token_in: { type: "asset", required: true }
  token_out: { type: "asset", required: true }
nodes:
  - id: "q_quote"
    type: "query_ref"
    protocol: "uniswap-v3@0.0.2"
    query: "quote"
    args:
      token_in: { ref: "inputs.token_in" }
      token_out: { ref: "inputs.token_out" }
  - id: "a_swap"
    type: "action_ref"
    protocol: "uniswap-v3@0.0.2"
    action: "swap_exact_in"
    deps: ["q_quote"]
    assert: { cel: "nodes.a_swap.outputs.tx_hash != ''" }
    assert_message: "swap must emit tx hash"
```
