# AIS-1: Core Schema

> Status: Draft
> Version: 1.0.0

## Abstract

AIS-1 defines the core schema for describing protocol interactions in a chain-agnostic way. AIS-1.0 enables Agents to "read chain and fill forms" and "construct interaction suggestions per spec" with interoperability, auditability, and extensibility.

## Design Principles

AIS-1.0 separates concerns into layers:

1. **Protocol Spec** describes atomic capabilities (actions, queries) and minimal execution recipes
2. **Pack** handles deployer selection, risk policies, available tokens and providers
3. **Workflow** handles cross-protocol composition and data flow orchestration
4. **Explicit query dependencies** — engine execution requires explicit query data dependencies; `condition` and `calculated` only reference query outputs and context
5. **CEL subset expressions** — capabilities can declare requirements

---

## Document Types

AIS-1.0 introduces three top-level document types, each independently versioned and cross-referenced.

### A) Protocol Skill Spec

Protocol capability definition, maintained by protocol authors.

```yaml
schema: "ais/1.0"

meta:
  protocol: string                    # Required. Protocol identifier (kebab-case).
  version: string                     # Required. Semver.
  name: string                        # Optional. Human-readable name.
  homepage: string                    # Optional. URL.
  logo: string                        # Optional. IPFS/Arweave URI.
  description: string                 # Optional. One-liner.
  tags: [string]                      # Optional. Categorization.
  maintainer: string                  # Optional. Contact/ENS.

deployments: [Deployment]             # Required. Where the protocol lives.
actions: { [id]: Action }             # Required. Write operations.
queries: { [id]: Query }              # Optional. Read-only operations.
risks: [Risk]                         # Optional. Protocol-level risk disclosures.
supported_assets: [AssetMapping]      # Optional. Recommended safe assets.
capabilities_required: [string]       # Optional. Required engine capabilities.
tests: [TestVector]                   # Optional. Lightweight test vectors.
```

### B) Pack

Deployment collection and policies, maintained by deployers.

```yaml
schema: "ais-pack/1.0"

name: string                          # Pack identifier.
version: string                       # Semver.
description: string                   # Optional.

includes: [string]                    # skill_id or skill_uri references.

policy:
  risk_threshold: integer             # Max risk_level to auto-approve.
  approval_required: [string]         # risk_tags requiring human approval.
  hard_constraints:                   # Default hard constraints.
    max_spend: string
    max_approval: string
    max_slippage_bps: integer
    allow_unlimited_approval: boolean

token_policy:
  allowlist: [string]                 # Allowed token symbols or addresses.
  resolution: "strict" | "permissive" # How to handle unknown tokens.

providers:
  quote: [string]                     # Enabled quote providers: oneinch, jupiter, etc.
  routing: [string]                   # Enabled routing providers.

overrides:                            # Per-skill overrides.
  [skill_id]:
    risk_tags: [string]               # Override risk tags.
    hard_constraints: object          # Override constraints.
```

### C) Workflow

Composite action orchestration, maintained by deployers or community.

```yaml
schema: "ais-flow/1.0"

meta:
  name: string                        # Workflow identifier.
  version: string                     # Semver.
  description: string                 # Optional.

requires_pack:                        # Optional. Pack dependency.
  name: string
  version: string

inputs:                               # Workflow input parameters.
  [param_name]:
    type: string
    required: boolean
    default: any                      # Optional.
    example: any                      # Optional.

nodes:                                # Execution nodes.
  - id: string                        # Node identifier for references.
    type: "query_ref" | "action_ref"  # Node type.
    skill: string                     # Skill reference (see below).
    query: string                     # Query ID (if type=query_ref).
    action: string                    # Action ID (if type=action_ref).
    args: object                      # Param mapping (references inputs.* or nodes.*.outputs.*).
    calculated_overrides: object      # Override calculated fields (CEL in workflow namespace).
    requires_queries: [string]        # Node IDs this node depends on.
    condition: string                 # Optional CEL condition.

policy:                               # Workflow-level policy (can tighten Pack policy).
  approvals: object
  hard_constraints: object

preflight:                            # Optional preflight simulation.
  simulate: object

outputs:                              # Workflow outputs.
  [output_name]: string               # Reference to nodes.*.outputs.* or nodes.*.calculated.*
```

#### Skill Reference Format

Workflows reference skills using `protocol@version` shorthand:

```yaml
skill: "uniswap-v3@1.0.0"
```

Pack is responsible for resolving `protocol@version` to registry `skillId` or `specURI`. Workflow authors only need to specify protocol name and version.

---

## Specification

### 1. Deployment

```yaml
deployments:
  - chain: string                     # Required. CAIP-2 chain ID.
    contracts:                        # Required. Named contract addresses.
      [name]: string                  # Address in chain-native format.
    rpc_hints: [string]               # Optional. Suggested RPC endpoints.
```

**Chain ID format:** [CAIP-2](https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-2.md)

| Chain | CAIP-2 ID |
|-------|-----------|
| Ethereum | `eip155:1` |
| Base | `eip155:8453` |
| Arbitrum | `eip155:42161` |
| Solana | `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp` |
| Cosmos Hub | `cosmos:cosmoshub-4` |
| Osmosis | `cosmos:osmosis-1` |
| Bitcoin | `bip122:000000000019d6689c085ae165831e93` |
| Aptos | `aptos:1` |
| Sui | `sui:mainnet` |

### 2. Action

An action is a state-changing operation that requires signing.

```yaml
actions:
  [action_id]:                        # kebab-case identifier
    description: string               # Required. What this does.
    risk_level: integer               # Required. 1 (safe) to 5 (dangerous).
    risk_tags: [string]               # Optional. Structured risk tags.
    
    params:                           # Required. Input parameters.
      - name: string                  # Required. Parameter name.
        type: string                  # Required. See Type System below.
        description: string           # Required. What this param means.
        required: boolean             # Optional. Default: true.
        default: any                  # Optional. Default value.
        constraints:                  # Optional. Validation rules.
          min: number
          max: number
          enum: [any]
          pattern: string             # Regex
    
    returns:                          # Optional. Expected outputs.
      - name: string
        type: string
        description: string
    
    requires_queries: [string]        # Required for complex actions. Query IDs.
    
    hard_constraints:                 # Optional. Action-level constraints.
      max_slippage_bps: string        # CEL expr or literal.
      max_spend: string
      max_approval: string
      allow_unlimited_approval: boolean
      max_price_impact_bps: integer
      min_health_factor_after: string # For lending protocols.
    
    calculated_fields:                # Structured calculated field declarations.
      [field_name]:
        expr: string                  # CEL expression.
        inputs: [string]              # Explicit query/param references.
    
    execution:                        # Required. Chain-specific details.
      [chain_pattern]: ExecutionSpec  # See AIS-2.
    
    pre_conditions: [string]          # Optional. Human-readable requirements.
    side_effects: [string]            # Optional. e.g., "Grants token approval".
```

**Risk Tags:**

| Tag | Description |
|-----|-------------|
| `approval` | Requires token approval |
| `unlimited_approval` | May request unlimited approval |
| `upgradeable` | Contract is upgradeable |
| `oracle_dependency` | Depends on price oracles |
| `mev_exposure` | Exposed to MEV extraction |
| `custody` | Protocol takes custody of funds |
| `irreversible` | Action cannot be undone |
| `external_bridge` | Crosses chain boundaries |
| `slippage` | Subject to slippage |

### 3. Query

A query is a read-only operation (no signing needed).

```yaml
queries:
  [query_id]:
    description: string
    params: [Param]                   # Same schema as Action params.
    returns: [ReturnField]
    
    cache_ttl: integer                # Optional. Suggested cache seconds.
    
    consistency:                      # Optional. Data consistency requirements.
      block_tag: "latest" | "safe" | "finalized" | integer
      require_same_block: boolean     # All reads in same block.
    
    execution:
      [chain_pattern]: ExecutionSpec
```

### 4. Risk

```yaml
risks:
  - level: "info" | "warning" | "critical"
    text: string                      # Human-readable risk description.
    applies_to: [string]              # Optional. Action IDs this risk applies to.
```

**Risk Level Resolution:**

`final_risk_level = max(protocol_declared, verifier_assessed, deployer_override)`

### 5. Asset Mapping (formerly Token Mapping)

Cross-chain asset address resolution with the new `asset` composite type.

```yaml
supported_assets:
  - symbol: string                    # e.g., "WETH"
    name: string                      # Optional. "Wrapped Ether"
    decimals:                         # Per-chain decimals.
      [chain_id]: integer
    addresses:
      [chain_id]: string              # Token address on each chain.
    coingecko_id: string              # Optional. For price feeds.
    tags: ["stable", "wrapped", "governance"]  # Optional.
```

---

## Type System

AIS uses a minimal type system for params and returns.

### Basic Types

| Type | Description | Example |
|------|-------------|---------|
| `address` | Chain-native address | `0x1234...` / `So1234...` |
| `uint256` | Unsigned 256-bit integer (as string) | `"1000000000000000000"` |
| `uint128` | Unsigned 128-bit integer | |
| `uint64` | Unsigned 64-bit integer | |
| `uint32` / `uint16` / `uint8` | Smaller unsigned integers | |
| `int256` | Signed 256-bit integer | |
| `bool` | Boolean | `true` / `false` |
| `string` | UTF-8 string | |
| `bytes` | Hex-encoded bytes | `"0xabcd..."` |
| `bytes32` | Fixed 32 bytes | |
| `float` | Floating point (for human-facing values) | `0.5` |
| `array<T>` | Array of type T | `array<address>` |
| `tuple<T1,T2,...>` | Ordered tuple | |

### Composite Types

#### `asset`

Represents a token/asset with full chain context.

```yaml
type: asset
# Structure:
# {
#   chain_id: string,     # CAIP-2 chain ID
#   address: string,      # Token contract address
#   symbol?: string,      # Optional symbol hint
#   decimals?: integer    # Optional decimals (engine fetches if missing)
# }
```

**Usage:**
```yaml
params:
  - name: token_in
    type: asset
    description: "Input token asset"
```

#### `token_amount`

Human-readable token amount. **Must be bound to an `asset` parameter.**

```yaml
params:
  - name: token_in
    type: asset
    description: "Input token"
  - name: amount_in
    type: token_amount
    description: "Human amount for token_in"
    # Implicitly bound to token_in (same prefix) or explicit binding:
    asset_ref: "token_in"
```

**Engine Responsibility:**

- Engine reads `decimals` from chain if not provided
- If decimals fetch fails, engine MUST reject `token_amount` conversion and require user to provide atomic amount
- Conversion: `atomic_amount = human_amount * 10^decimals`

---

## Expression Language (CEL Profile)

AIS-1.0 uses a restricted [CEL (Common Expression Language)](https://github.com/google/cel-spec) subset for `condition` and `calculated_fields`.

### Expression Namespaces

**Skill Spec and Workflow use separate expression namespaces:**

#### Skill Spec Namespace (actions/queries)

`calculated_fields` and `condition` in Protocol Specs only reference:

| Variable | Description |
|----------|-------------|
| `params.*` | Action/query parameters |
| `ctx.wallet_address` | Signer's address |
| `ctx.chain_id` | Current chain (CAIP-2) |
| `ctx.now` | Current Unix timestamp (seconds) |
| `ctx.policy.*` | Active policy constraints |
| `query.<query_id>.<field>` | Query output fields |
| `contracts.<name>` | Deployment contract addresses |
| `calculated.<field>` | Other calculated fields (within same action) |

#### Workflow Namespace (nodes)

`calculated_overrides` and `condition` in Workflows only reference:

| Variable | Description |
|----------|-------------|
| `inputs.*` | Workflow input parameters |
| `nodes.<id>.outputs.*` | Output fields from prior nodes |
| `nodes.<id>.calculated.*` | Calculated fields from prior nodes |

**Mapping Rules:**

When a workflow invokes a skill action:
- Workflow `inputs.*` → mapped to action `params.*` via `args`
- Workflow `nodes.<id>.outputs.*` → mapped to action `query.<id>.*` via `args` or `calculated_overrides`

This separation ensures:
1. Skill specs are self-contained and testable
2. Workflows are composable without namespace collisions
3. Expression evaluation context is always explicit

### Allowed Operations

- **Arithmetic:** `+`, `-`, `*`, `/`, `%`
- **Comparison:** `==`, `!=`, `<`, `<=`, `>`, `>=`
- **Logical:** `&&`, `||`, `!`
- **Conditional:** `condition ? true_value : false_value`
- **Functions:** `min`, `max`, `abs`, `ceil`, `floor`, `round`
- **Type conversion:** `to_atomic(amount, asset)`, `to_human(atomic, asset)`

### Prohibited

- Reflection or dynamic evaluation
- Loops or recursion
- String concatenation to generate addresses, function names, or ABI
- External calls or side effects

**Example:**
```yaml
calculated_fields:
  amount_in_atomic:
    expr: "to_atomic(params.amount_in, params.token_in)"
  min_out_atomic:
    expr: "floor(query.quote.amount_out_atomic * (1.0 - params.slippage_bps / 10000.0))"
  deadline_unix:
    expr: "ctx.now + 600"
```

---

## Structured Detection

The `auto_detect` string is replaced with a structured `detect` object.

```yaml
mapping:
  fee: 
    detect:
      kind: "choose_one" | "best_quote" | "best_path" | "protocol_specific"
      provider: string              # oneinch, jupiter, internal_router, etc.
      candidates: [any]             # Candidate values to choose from.
      constraints: object           # Provider-specific constraints.
      requires_capabilities: [string]  # Engine capabilities needed.
```

**Detection Kinds:**

| Kind | Description |
|------|-------------|
| `choose_one` | Select one from candidates based on criteria |
| `best_quote` | Query provider for best quote |
| `best_path` | Find optimal routing path |
| `protocol_specific` | Protocol-defined detection logic |

---

## Capabilities

Engine implementations vary. AIS-1.0 introduces capability declarations.

### Skill Declaration

```yaml
capabilities_required:
  - "cel_v1"            # CEL expression support
  - "evm_multiread"     # Multicall support
  - "permit2"           # Permit2 signature support
  - "quote_provider:oneinch"  # Specific quote provider
```

### Engine Advertisement

Engines report capabilities at startup. If a skill requires capabilities the engine doesn't support:
1. Execution is rejected with clear error, OR
2. Engine falls back to explicit degradation path

---

## Chain Pattern Matching

Execution blocks use glob patterns for chain matching:

| Pattern | Matches |
|---------|---------|
| `eip155:1` | Ethereum mainnet only |
| `eip155:*` | All EVM chains |
| `solana:*` | All Solana clusters |
| `cosmos:osmosis-1` | Osmosis only |
| `*` | Fallback (any chain) |

Resolution order: most specific pattern first.

---

## Versioning

- Spec files use semver: `MAJOR.MINOR.PATCH`
- **MAJOR:** Breaking changes to params or execution
- **MINOR:** New actions/queries, non-breaking additions
- **PATCH:** Description/metadata fixes
- Registry enforces: major version bump → re-verification required

---

## Test Vectors (Optional)

Skills can include lightweight test vectors for validation.

```yaml
tests:
  - name: "swap-basic"
    action: "swap-exact-in"
    params:
      token_in: { chain_id: "eip155:8453", address: "0x4200...", decimals: 18 }
      token_out: { chain_id: "eip155:8453", address: "0x8330...", decimals: 6 }
      amount_in: "1.0"
      slippage_bps: 50
    expect:
      calculated:
        amount_in_atomic: "1000000000000000000"
      execution_type: "composite"
```

---

## Examples

See [/examples](../examples/) for complete spec files.

---

## Rationale

**Why YAML over JSON?** — Human readability matters for a spec that protocol teams will write and maintain. YAML supports comments and is less noisy.

**Why CAIP-2?** — Industry standard for chain identification. Avoids inventing yet another chain ID scheme.

**Why risk_level as integer?** — Enables programmatic policy decisions ("auto-approve risk ≤ 2").

**Why asset type?** — Resolves symbol/address ambiguity. Agents often see "swap USDC for ETH" but need precise addresses. The `asset` type bundles chain, address, and decimals.

**Why CEL?** — Well-specified, sandboxable, already used in IAM policies. Avoids inventing yet another expression language.

**Why capabilities?** — Different engines have different features. Explicit capability negotiation prevents runtime surprises.
