# AIS — Agent Interaction Spec

A standard for describing DeFi protocol interfaces in a way that AI agents can safely consume and execute.

## Problem

AI agents need to interact with on-chain protocols, but:
- Hardcoding protocol knowledge is fragile and unscalable
- Fetching instructions from URLs is vulnerable to phishing/tampering
- Each chain has a different execution model (EVM calls, Solana instructions, Cosmos messages...)
- Agents need risk information to make safe decisions

## Solution

AIS defines a **chain-agnostic schema** for protocol interaction specs:
- Standardized action descriptions with typed parameters
- Chain-specific execution details (how to actually build the transaction)
- Risk metadata for policy engines
- On-chain registry for verifiable, tamper-proof distribution

## Quick Example

```yaml
schema: "ais/1.0"
protocol: "uniswap-v3"
version: "1.2.0"

actions:
  swap:
    description: "Swap one token for another"
    risk_level: 2
    params:
      - { name: token_in, type: address, description: "Token to sell" }
      - { name: token_out, type: address, description: "Token to buy" }
      - { name: amount, type: uint256, description: "Amount in wei" }
    execution:
      "eip155:*":
        type: evm_call
        contract: "router"
        function: "exactInputSingle"
```

## Spec Documents

- [AIS-1: Core Schema](./specs/ais-1-core.md) — Metadata, actions, params, queries
- [AIS-2: Execution Types](./specs/ais-2-execution.md) — Chain-specific execution formats
- [AIS-3: Registry](./specs/ais-3-registry.md) — On-chain registry contract & governance

## Design Principles

1. **Chain-agnostic params, chain-specific execution** — Agent thinks in intents, engine handles chain details
2. **Verifiable by default** — Specs live on IPFS, hashes live on-chain
3. **Progressive decentralization** — Start centralized, evolve to community governance
4. **Agent-first, human-readable** — YAML for readability, strict schema for parsing

## Status

🚧 Draft — Feedback welcome

## License

CC0 1.0 Universal
