# AIS-0: Overview (v0.0.2)

Status: Draft  
Spec Version: 0.0.2  

AIS (Agent Interaction Spec) is a set of YAML documents that allow agents/engines to:

1) **Discover** protocol interaction specs (Registry/Discovery)  
2) **Validate** specs and user inputs (Schema/Policy)  
3) **Plan & Build** chain-specific executions (Execution)  

## Design goals (0.0.2)

- **Unambiguous**: no “string that might mean literal or reference”.
- **Deterministic**: encoding does not depend on YAML map iteration order.
- **Precise numeric model**: no IEEE754 for on-chain amounts.
- **Cross-chain composite**: the same step container works for EVM/Solana/etc.
- **Conformance-first**: every MUST has a test vector.

## Document schemas (0.0.2)

- Protocol Spec: `schema: "ais/0.0.2"`
- Pack: `schema: "ais-pack/0.0.2"`
- Workflow: `schema: "ais-flow/0.0.3"`

## Key concepts

- **ValueRef**: a structured value expression used everywhere inputs may be dynamic (references, CEL, detection).
- **JSON ABI (EVM)**: function ABI is expressed as a JSON fragment, enabling tuple/struct encoding.
- **Composite**: a generic multi-step container whose steps embed chain-specific `ExecutionSpec`.

## Minimal runtime context (T431)

Engines evaluate `ValueRef` (`ref`/`cel`) against a runtime root object:

```yaml
inputs: {}      # workflow inputs (user-provided)
params: {}      # per-node params (derived from nodes[].args; injected by planner/executor)
ctx: {}         # environment (wallet, time, etc.)
contracts: {}   # deployment contracts (can be auto-filled by solver from protocol deployments)
nodes: {}       # per-node state (engine writes outputs here)
query: {}       # optional legacy/flat query bag (prefer nodes.<id>.outputs)
calculated: {}  # optional computed values (engine-defined)
policy: {}      # optional pack/workflow policy values
```

In practice, most workflows require:
- `inputs.*` (matching `workflow.inputs`)
- `ctx.wallet_address`
- `contracts.*` for any `contracts.<name>` references
