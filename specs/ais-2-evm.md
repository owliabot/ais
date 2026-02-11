# AIS-2E: EVM Execution — v0.0.2

Status: Draft  
Spec Version: 0.0.2  

This document defines EVM execution specs for `eip155:*`.

## 0. Chain pattern matching (ExecutionBlock)

`execution` fields in protocol specs and actions/queries are **ExecutionBlock** maps:

```yaml
execution:
  "eip155:1": { ... }     # exact CAIP-2 chain id
  "eip155:*": { ... }     # namespace wildcard
  "*": { ... }            # global fallback
```

Matching algorithm (normative):

1) Try exact match: `execution[chain]`
2) Try namespace wildcard: `execution["<namespace>:*"]` where `<namespace>` is the part before `:`
3) Try global wildcard: `execution["*"]`
4) If no match, engines MUST error (no implicit defaults)

Conflict rule (normative):
- If multiple keys could match, engines MUST use the highest-precedence rule above (exact > namespace wildcard > global).
- Duplicate keys in YAML documents MUST be rejected (engines MUST NOT “last one wins” silently).

## 1. JSON ABI fragment (D2=A)

AIS 0.0.2 uses a JSON ABI fragment for functions:

```yaml
abi:
  type: "function"
  name: "exactInputSingle"
  inputs:
    - { name: "params", type: "tuple", components: [ ... ] }
  outputs: []
```

Engines MUST encode tuples/structs according to ABI `components`.

## 2. `evm_call`

```yaml
type: evm_call
to: { ref: "contracts.router" }       # ValueRef resolving to address
abi: <JsonAbiFunction>
args:
  spender: { ref: "contracts.router" } # keyed by input name
  amount: { ref: "calculated.amount" }
value: { lit: "0" }                    # optional; uint as decimal string
```

Rules:
- `args` MUST be keyed by ABI input name.
- Engines MUST reject missing/extra args.
- All integer args MUST be encoded from BigInt (or decimal-string converted to BigInt).

## 3. `evm_read`

Same shape as `evm_call` but indicates `eth_call` (no signing).

Return decoding (normative):
- Engines MUST decode `eth_call` return data using the JSON ABI `abi.outputs`.
- `abi.outputs[*].name` MUST be non-empty and unique.
- When used in a **query** (`queries.*`), `abi.outputs` MUST match `query.returns` (see `specs/ais-1-protocol.md`).

## 4. `evm_multiread`

```yaml
type: evm_multiread
method: "multicall3" | "rpc_batch"
calls:
  - id: "allowance"
    to: { ref: "params.token.address" }
    abi: <JsonAbiFunction>
    args: { ... }
```

## 5. `evm_multicall`

Atomic batching for routers that support multicall-like patterns.

```yaml
type: evm_multicall
to: { ref: "contracts.router" }
calls:
  - abi: <JsonAbiFunction>
    args: { ... }
    condition: { cel: "..." }         # optional
deadline: { ref: "calculated.deadline" }  # optional
```

## 6. Plugin execution type: `evm_rpc` (non-core)

Some EVM information is only exposed via native JSON-RPC methods (e.g. `eth_getBalance`).
AIS core keeps the execution surface small; engines MAY expose a controlled RPC escape hatch as a **plugin** execution type.

This repository's TypeScript SDK ships a built-in plugin execution type `evm_rpc` that is:
- validated via the execution plugin registry
- gated by packs under `plugins.execution.enabled` when a pack is in use
- executor-enforced to a read-only allowlist of safe methods

Shape (informative):

```yaml
type: evm_rpc
method: "eth_getBalance"   # executor allowlist; other methods may be rejected
params:
  array:
    - { lit: "0x..." }     # address
    - { lit: "latest" }   # block tag (or hex quantity)
```
