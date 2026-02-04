# AIS-1: Core Schema

> Status: Draft
> Version: 1.0.0

## Abstract

AIS-1 defines the core schema for describing protocol interactions in a chain-agnostic way. It covers metadata, deployments, actions, queries, risk declarations, and token mappings.

## Specification

### 1. Top-Level Structure

```yaml
schema: "ais/1.0"                    # Required. Schema version.

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
supported_tokens: [TokenMapping]      # Optional. Recommended safe tokens.
```

### 2. Deployment

```yaml
deployments:
  - chain: string                     # Required. CAIP-2 chain ID.
    contracts:                        # Required. Named contract addresses.
      [name]: string                  # Address in chain-native format.
    rpc_hints: [string]               # Optional. Suggested RPC endpoints.
```

**Chain ID format:** [CAIP-2](https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-2.md)
- EVM: `eip155:1` (Ethereum), `eip155:8453` (Base), `eip155:42161` (Arbitrum)
- Solana: `solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp`
- Cosmos: `cosmos:cosmoshub-4`, `cosmos:osmosis-1`
- Bitcoin: `bip122:000000000019d6689c085ae165831e93`
- Aptos: `aptos:1`
- Sui: `sui:mainnet`

### 3. Action

An action is a state-changing operation that requires signing.

```yaml
actions:
  [action_id]:                        # kebab-case identifier
    description: string               # Required. What this does.
    risk_level: integer               # Required. 1 (safe) to 5 (dangerous).
    
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
    
    execution:                        # Required. Chain-specific details.
      [chain_pattern]: ExecutionSpec  # See AIS-2.
    
    pre_conditions: [string]          # Optional. Human-readable requirements.
    side_effects: [string]            # Optional. e.g., "Grants token approval".
```

### 4. Query

A query is a read-only operation (no signing needed).

```yaml
queries:
  [query_id]:
    description: string
    params: [Param]                   # Same schema as Action params.
    returns: [ReturnField]
    execution:
      [chain_pattern]: ExecutionSpec
    cache_ttl: integer                # Optional. Suggested cache seconds.
```

### 5. Risk

```yaml
risks:
  - level: "info" | "warning" | "critical"
    text: string                      # Human-readable risk description.
    applies_to: [string]              # Optional. Action IDs this risk applies to.
```

### 6. Token Mapping

Cross-chain token address resolution.

```yaml
supported_tokens:
  - symbol: string                    # e.g., "WETH"
    name: string                      # Optional. "Wrapped Ether"
    decimals:                         # Per-chain decimals (usually same, not always)
      [chain_id]: integer
    addresses:
      [chain_id]: string             # Token address on each chain.
    coingecko_id: string              # Optional. For price feeds.
    tags: ["stable", "wrapped", "governance"]  # Optional.
```

### 7. Type System

AIS uses a minimal type system for params and returns:

| Type | Description | Example |
|---|---|---|
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
| `float` | Floating point (for human-facing values like slippage %) | `0.5` |
| `token_amount` | Human-readable token amount (engine handles decimals) | `"1.5"` |
| `array<T>` | Array of type T | `array<address>` |
| `tuple<T1,T2,...>` | Ordered tuple | |

**Note:** `token_amount` is a convenience type. When `type: token_amount`, the engine converts human amounts to on-chain representation using the token's decimals.

### 8. Chain Pattern Matching

Execution blocks use glob patterns for chain matching:

| Pattern | Matches |
|---|---|
| `eip155:1` | Ethereum mainnet only |
| `eip155:*` | All EVM chains |
| `solana:*` | All Solana clusters |
| `cosmos:osmosis-1` | Osmosis only |
| `*` | Fallback (any chain) |

Resolution order: most specific pattern first.

### 9. Versioning

- Spec files use semver: `MAJOR.MINOR.PATCH`
- MAJOR: breaking changes to params or execution
- MINOR: new actions/queries, non-breaking
- PATCH: description/metadata fixes
- Registry enforces: major version bump → re-verification required

## Examples

See [/examples](../examples/) for complete spec files.

## Rationale

**Why YAML over JSON?** — Human readability matters for a spec that protocol teams will write and maintain. YAML supports comments and is less noisy. Parsers convert to JSON internally.

**Why CAIP-2?** — Industry standard for chain identification. Avoids inventing yet another chain ID scheme.

**Why risk_level as integer?** — Enables programmatic policy decisions. A policy engine can say "auto-approve risk ≤ 2, require human approval for risk ≥ 4" without parsing text.

**Why token_amount type?** — Agents think in human units ("swap 1.5 ETH"). Forcing uint256 wei values into the spec would make it harder for both agents and humans to reason about.
