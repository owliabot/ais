# AIS-2X: Composite Execution — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

`composite` is a cross-chain multi-step container used to express:

- EVM: approve → swap
- Solana: create ATA → transfer

## 1. Shape

```yaml
execution:
  "<chain_pattern>":
    type: composite
    steps:
      - id: "<step_id>"
        description: "..."
        chain: "eip155:1"             # optional; per-step chain override (inherits parent if omitted)
        condition: { cel: "..." }     # optional; if false, step is skipped
        execution: <ExecutionSpec>    # REQUIRED
```

## 2. Semantics

- Steps execute in order, skipping steps whose condition evaluates to false.
- Conditions MUST be pure/deterministic.
- If `steps[].chain` is present, that step executes on the specified chain (useful for cross-chain bridge actions).
- The evaluation context for each step is the same as the action context, plus:
  - `calculated.*` that were computed before execution planning
  - `query.*` results required by the action (if provided by the engine)

Engines MUST NOT allow conditions to depend on “transaction results of previous steps” unless such results are explicitly modeled as outputs.
