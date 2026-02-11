# AIS-1C: Workflow — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Workflows orchestrate cross-protocol composition. All dynamic values MUST use `ValueRef`.

## 0. Strict fields and `extensions`

AIS 0.0.2 workflow objects are **strict**:
- Unknown fields MUST be rejected.
- Extensions MUST live under `extensions` (free-form, implementation-defined).

Extensions MAY appear at:
- the workflow root (`workflow.extensions`)
- individual nodes (`nodes[].extensions`)

## 0. Chain selection (multi-chain)

Each workflow node MUST resolve to exactly one CAIP-2 chain id.

Chain inheritance:
- `nodes[].chain` (highest precedence)
- `workflow.default_chain`

Rationale:
- The same workflow MAY include nodes for multiple chains (e.g. EVM + Solana).
- Protocol `execution` blocks MAY use wildcards (e.g. `eip155:*`), but workflow nodes MUST resolve to a concrete chain id so the engine can route RPCs/signing correctly.

## 1. Node dependencies (DAG)

Workflow `nodes[]` form a directed acyclic graph (DAG).

- A node MAY declare explicit dependencies via `deps: ["<node_id>", ...]`.
- A node also has implicit dependencies if its `args` / `condition` reference `nodes.<id>.*`.
- Engines MUST reject cycles.

Scheduling semantics (recommended):
- Nodes whose dependencies are satisfied MAY be executed in parallel.
- For EVM writes from the same account on the same chain, engines SHOULD serialize broadcast (nonce order),
  while allowing reads to run in parallel.

```yaml
schema: "ais-flow/0.0.2"
meta: { name: "...", version: "...", description: "..." }
default_chain: "eip155:1"
inputs: { ... }
nodes:
  - id: "q_quote"
    type: "query_ref"
    protocol: "uniswap-v3@0.0.2"
    query: "quote"
    args:
      token_in: { ref: "inputs.token_in" }
  - id: "a_swap"
    type: "action_ref"
    protocol: "uniswap-v3@0.0.2"
    action: "swap"
    deps: ["q_quote"]
  - id: "q_solana_balance"
    type: "query_ref"
    chain: "solana:mainnet"
    protocol: "spl-token@0.0.2"
    query: "token-balance"
outputs:
  min_out: { ref: "nodes.a_swap.calculated.min_out" }
```

## 2. Polling / until (engine-driven)

Nodes MAY declare post-check polling fields:
- `until`: `ValueRef` evaluated **after** the node executes successfully. If the result is falsy, the node is not considered completed and MAY be retried.
- `retry`: `{ interval_ms, max_attempts?, backoff? }` (default backoff is fixed).
- `timeout_ms`: overall timeout for the node's polling lifecycle.

Recommended usage:
- Use `until` + `retry` for “wait until arrived/confirmed” queries (bridge arrival, balance increased, receipt exists, etc).
- Avoid inventing new execution types for waiting; keep waiting semantics in workflow fields.
