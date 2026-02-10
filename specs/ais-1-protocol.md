# AIS-1A: Protocol Spec — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

Protocol Specs describe a protocol’s actions/queries and execution recipes.

## 0. Strict fields and `extensions`

AIS 0.0.2 schemas are **strict**:
- Unknown fields MUST be rejected.
- Implementations MUST NOT silently ignore or strip unknown keys.

Extensibility:
- If an engine/registry/agent needs to attach extra metadata, it MUST do so under an `extensions` object.
- `extensions` is a free-form map whose contents are implementation-defined and MUST NOT affect execution semantics unless explicitly standardized later.

Example:

```yaml
schema: "ais/0.0.2"
meta:
  protocol: "uniswap-v3"
  version: "0.0.2"
  extensions:
    ui:
      color: "purple"
actions:
  swap:
    description: "..."
    risk_level: 3
    extensions:
      docs: { url: "..." }
```

Top-level:

```yaml
schema: "ais/0.0.2"
meta: { protocol: "...", version: "...", name: "..." }
deployments: [...]
actions: { ... }
queries: { ... }
```

## 1. Top-level fields

```yaml
schema: "ais/0.0.2"

meta:
  protocol: "uniswap-v3"              # kebab-case id
  version: "0.0.2"                    # spec author version (semver)
  name: "Uniswap V3"                  # optional
  homepage: "https://..."             # optional
  description: "..."                  # optional
  tags: ["dex"]                       # optional
  maintainer: "..."                   # optional

capabilities_required: ["cel:v1"]     # optional

deployments:
  - chain: "eip155:1"                 # CAIP-2
    contracts: { router: "0x..." }    # named addresses

supported_assets: [ ... ]             # optional (multi-chain mapping)
risks: [ ... ]                        # optional (protocol-level risk notes)

actions: { "<action_id>": <Action> }  # REQUIRED
queries: { "<query_id>": <Query> }    # optional
tests: [ ... ]                        # optional
```

## 2. Action

```yaml
actions:
  swap-exact-in:
    description: "..."
    risk_level: 3
    risk_tags: ["slippage", "approval"]   # open string set; lint may recommend

    params:
      - name: token_in
        type: asset
        description: "..."
      - name: amount_in
        type: token_amount
        asset_ref: "token_in"
        description: "..."
        required: true
        default: null
        constraints: { min: 0 }

    returns:
      - { name: tx_hash, type: string }

    requires_queries: ["quote", "allowance"]

    hard_constraints:
      max_slippage_bps: { ref: "params.slippage_bps" }
      allow_unlimited_approval: { lit: false }

    calculated_fields:
      amount_in_atomic:
        expr: { cel: "to_atomic(params.amount_in, params.token_in)" }
        inputs: ["params.amount_in", "params.token_in"]

    execution:
      "eip155:*": <ExecutionSpec>          # see AIS-2
```

Notes:
- `calculated_fields[*].expr` is a **ValueRef** (typically `{cel:"..."}`).
- All action/query execution specs are defined in AIS-2.

## 3. Query

Queries are read-only and share the same `params/returns/execution` structure as actions, but have no `risk_level`.

Details are defined across:
- Types: `specs/ais-1-types.md`
- Expressions: `specs/ais-1-expressions.md`
- Execution: `specs/ais-2-*.md`

### 3.1 `returns` and output writing (normative)

`queries[*].returns` defines the **canonical output object** shape for query results.

- Engines MUST write query results as an object keyed by `returns[*].name`.
- For workflow `query_ref` nodes, engines MUST write outputs to `nodes.<workflowNodeId>.outputs` (not to `query.<id>`), to avoid ambiguity across multiple node instances.

EVM binding rule (normative):
- If a query uses `evm_read`, its `returns` MUST match the JSON ABI `abi.outputs` **exactly**:
  - same length
  - same order
  - same names (`returns[*].name === abi.outputs[*].name`)
  - same types (`returns[*].type === abi.outputs[*].type`)
- Engines MUST reject empty/unnamed `abi.outputs` when the query declares `returns`.

Tuple rule (normative):
- `returns` maps only **top-level** ABI outputs.
- If an ABI output is a tuple, engines MUST decode it as:
  - an object when all tuple component names are present and unique, otherwise
  - an array (positional)
  and store it under that single return name (no implicit flattening into multiple returns).
