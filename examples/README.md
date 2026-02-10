# Examples (AIS 0.0.2)

This directory contains **reference** Protocol specs (`.ais.yaml`), Packs (`.ais-pack.yaml`), and Workflows (`.ais-flow.yaml`) used for testing and documentation.

## Minimal runtime context template (T431)

AIS uses a runtime root object for `ref` and `cel` evaluation:

```yaml
inputs:      # workflow inputs (user-provided)
params:      # per-node params (derived from nodes[].args; injected by planner/executor)
ctx:         # environment (wallet address, time, etc.)
contracts:   # deployment contracts (can be auto-filled by solver from protocol deployments)
nodes:       # engine-managed per-node state (outputs written here)
query:       # optional legacy/flat query bag (avoid for workflows; prefer nodes.<id>.outputs)
calculated:  # optional computed values (only if your engine implements it)
policy:      # optional pack/workflow policy values
```

**Required in practice**
- `inputs.*` — must match `workflow.inputs` keys.
- `ctx.wallet_address` — used by most on-chain actions/queries.
- `contracts.*` — required when a spec references `contracts.<name>`; the built-in solver may auto-fill this from `deployments[].contracts` for the node chain.

**Recommended conventions**
- Prefer `nodes.<id>.outputs.*` over `query["..."]` for workflow composition.
- Use unique `contracts` keys across protocols to avoid collisions (contracts live in a shared bag).

## Notable examples

- `bridge-send-wait-deposit.ais-flow.yaml`: EVM send → poll Solana arrival → Solana deposit
- `aave-branch-bridge-solana-deposit.ais-flow.yaml`: Aave borrow → branch transfer + bridge → wait → deposit

