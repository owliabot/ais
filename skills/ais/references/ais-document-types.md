# AIS Document Types

AIS documents are strict JSON or YAML files identified by a `schema:` field.

All six document families are strict: unknown fields are rejected unless carried in an explicit `extensions` field.

## 1) Protocol
- Schema ID: `ais/0.0.2`
- Purpose: define one protocol's actions, queries, deployments, and metadata.
- Key fields: `schema`, `meta`, `deployments`, `actions`, optional `queries`, `capabilities_required`, `supported_assets`, `risks`, `tests`.

Example:
```yaml
schema: "ais/0.0.2"
meta: { protocol: "example-protocol", version: "0.0.2" }
actions:
  transfer:
    description: "Transfer tokens"
    risk_level: 3
    params:
      - name: recipient
        type: address
        description: "Recipient address"
      - name: amount
        type: uint256
        description: "Amount in atomic units"
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.token" }
        abi:
          type: "function"
          name: "transfer"
          inputs:
            - { name: "to", type: "address" }
            - { name: "value", type: "uint256" }
          outputs: []
        args:
          to: { ref: "params.recipient" }
          value: { ref: "params.amount" }
```

## 2) Pack
- Schema ID: `ais-pack/0.0.2`
- Purpose: bundle protocol includes and policy controls.
- Key fields: `schema`, `meta`, `includes`, `policy`, optional `token_policy`, `providers`, `plugins`, `overrides`.

Example:
```yaml
schema: "ais-pack/0.0.2"
meta: { name: "safe-defi-pack", version: "0.0.2" }
includes:
  - { protocol: "uniswap-v3", version: "0.0.2", source: "registry" }
policy:
  approvals: { mode: "safe", require_approval_min_risk_level: 3 }
```

## 3) Workflow
- Schema ID: `ais-flow/0.0.3`
- Purpose: DAG orchestration over query/action operations.
- Key fields: `schema`, `meta`, `nodes`, optional `default_chain`, `imports`, `requires_pack`, `inputs`, `policy`, `preflight`, `outputs`.
- Node `type`: `query_ref | action_ref | assert | branch`.

Example:
```yaml
schema: "ais-flow/0.0.3"
meta: { name: "swap-with-guard", version: "0.0.1" }
default_chain: "eip155:1"
nodes:
  - id: q_quote
    type: query_ref
    protocol: uniswap-v3@0.0.2
    query: quote
```

## 4) Plan
- Schema ID: `ais-plan/0.0.3`
- Purpose: runner/engine execution contract.
- Key fields: root `schema`, `nodes`, optional `meta`, `extensions`; node essentials `id`, `chain`, `kind`, `execution` plus optional controls (`deps`, `condition`, `assert`, `until`, `retry`, `timeout_ms`).

Example:
```yaml
schema: "ais-plan/0.0.3"
nodes:
  - id: n1
    chain: eip155:1
    kind: action_ref
    execution: { type: evm_call }
```

## 5) Plan Sketch
- Schema ID: `ais-plan-sketch/0.1.0`
- Purpose: LLM-facing segmented planning IR; must be compiled to `ais-plan/0.0.3` before execution.
- Key fields: `schema`, `intent`, `pack_snapshot`, `catalog_snapshot`, `segments[]`; each segment carries `segment_id`, `cursor_in`, `cursor_out`, `done`, `steps[]`.

Example:
```yaml
schema: "ais-plan-sketch/0.1.0"
intent: "swap 1 ETH to USDC"
pack_snapshot: { name: safe-defi-pack, version: "0.0.2", hash: "..." }
catalog_snapshot: { hash: "..." }
segments:
  - segment_id: s1
    cursor_in: "0"
    cursor_out: "1"
    done: false
    steps:
      - { id: step1, kind: query, candidate_ref: "uniswap-v3@0.0.2/quote", inputs: {} }
```

## 6) Catalog
- Schema ID: `ais-catalog/0.0.1`
- Purpose: index-only discovery cards for actions/queries/packs.
- Key fields: `schema`, `created_at`, `hash`, optional `documents`, plus `actions[]`, `queries[]`, `packs[]`.

Example:
```yaml
schema: "ais-catalog/0.0.1"
created_at: "2026-03-04T00:00:00Z"
hash: "sha256:..."
actions:
  - ref: uniswap-v3@0.0.2/swap_exact_in
    protocol: uniswap-v3
    version: 0.0.2
    id: swap_exact_in
    risk_level: 3
```
