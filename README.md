# AIS — Agent Interaction Spec

A standard for describing DeFi protocol interfaces in a way that AI agents can safely consume and execute.

## Problem

AI agents need to interact with on-chain protocols, but:
- Hardcoding protocol knowledge is fragile and unscalable
- Fetching instructions from URLs is vulnerable to phishing/tampering
- Each chain has a different execution model (EVM calls, Solana instructions, Cosmos messages...)
- Agents need risk information to make safe decisions
- Symbol/address ambiguity causes dangerous mismatches

## Solution

AIS defines a **chain-agnostic schema** for protocol interaction specs:
- **Protocol Specs** — Standardized actions and queries with typed parameters
- **Packs** — Deployment bundles with risk policies and constraints
- **Workflows** — Cross-protocol composition and data flow
- **Registry** — On-chain verification for tamper-proof distribution

## Quick Example

```yaml
schema: "ais/1.0"

meta:
  protocol: "uniswap-v3"
  version: "1.0.0"
  name: "Uniswap V3"

actions:
  swap-exact-in:
    description: "Swap exact input for maximum output"
    risk_level: 3
    risk_tags: ["mev_exposure", "slippage"]
    
    params:
      - name: token_in
        type: asset                   # New composite type
        description: "Input token"
      - name: token_out
        type: asset
        description: "Output token"
      - name: amount_in
        type: token_amount            # Bound to token_in
        description: "Amount to swap"
      - name: slippage_bps
        type: uint32
        description: "Max slippage in basis points"
    
    requires_queries:                 # Explicit dependencies
      - "quote-exact-in-single"
      - "allowance-token-in"
    
    calculated_fields:                # Structured calculations
      amount_in_atomic:
        expr: "to_atomic(params.amount_in, params.token_in)"
      min_out_atomic:
        expr: "floor(query.quote-exact-in-single.amount_out_atomic * (1.0 - params.slippage_bps / 10000.0))"
    
    execution:
      "eip155:*":
        type: composite
        steps:
          - id: approve
            type: evm_call
            condition: "query.allowance-token-in.allowance_atomic < calculated.amount_in_atomic"
            # ...
          - id: swap
            type: evm_call
            # ...
```

## Spec Documents

| Document | Description |
|----------|-------------|
| [AIS-1: Core Schema](./specs/ais-1-core.md) | Protocol specs, packs, workflows, type system, CEL expressions |
| [AIS-2: Execution Types](./specs/ais-2-execution.md) | Chain-specific execution formats (EVM, Solana, Cosmos, etc.) |
| [AIS-3: Registry](./specs/ais-3-registry.md) | On-chain registry, discovery layer, governance |

## Design Principles

1. **Layered Architecture**
   - Protocol authors write capabilities (Spec)
   - Deployers select and constrain (Pack)
   - Orchestrators compose (Workflow)

2. **Explicit Data Dependencies**
   - `requires_queries` declares what chain reads are needed
   - `calculated_fields` shows how derived values are computed
   - No hidden inference or magic

3. **CEL Expressions**
   - Sandboxed, well-specified expression language
   - Conditions and calculations only — no dynamic contract generation

4. **Chain-Agnostic, Chain-Specific**
   - Params are intent-level (human amounts, asset references)
   - Execution is chain-specific (EVM calls, Solana instructions)

5. **Verifiable by Default**
   - Specs live on IPFS, hashes live on-chain
   - Domain verification for authority binding

6. **Capability Negotiation**
   - Skills declare requirements
   - Engines advertise capabilities
   - Clear failures when incompatible

## Key Concepts

### Asset Type

Resolves symbol/address ambiguity:

```yaml
params:
  - name: token_in
    type: asset
    # { chain_id, address, symbol?, decimals? }
```

### Hard Constraints

Enforceable limits at multiple levels:

```yaml
hard_constraints:
  max_slippage_bps: 200
  max_approval: "params.amount_in"
  allow_unlimited_approval: false
```

### Risk Tags

Structured risk categorization for policy engines:

```yaml
risk_tags:
  - mev_exposure
  - slippage
  - approval
```

## Examples

See [/examples](./examples/) for complete spec files:

**Protocol Specs:**
- `uniswap-v3.ais.yaml` — DEX swap with quote-based slippage
- `aave-v3.ais.yaml` — Lending protocol supply/withdraw

**Pack (Deployment Bundle):**
- `safe-defi-pack.ais-pack.yaml` — Conservative policy pack for Uniswap V3 on Base

**Workflow (Orchestration):**
- `swap-to-token.ais-flow.yaml` — Quote-then-swap workflow using Uniswap V3

## Status

🚧 Draft v1.0.0 — Feedback welcome

## License

CC0 1.0 Universal
