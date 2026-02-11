# Execution / EVM (AIS 0.0.2)

EVM-specific execution helpers:

- Keccak256 selector hashing
- ABI encoding/decoding via `ethers` (JSON ABI, tuple-safe)
- Compilation of `evm_read` / `evm_call` specs into `{to,data,value,chainId}`
- Compilation support for the built-in plugin execution type `evm_rpc` (method + params)

This submodule performs **no network IO**.

## File Structure

| File | Purpose |
|------|---------|
| `keccak.ts` | `keccak256()` helper |
| `encoder.ts` | JSON ABI encode/decode helpers (ethers-backed) |
| `compiler.ts` | `compileEvmExecution()` / `compileEvmExecutionAsync()` |
| `index.ts` | Re-exports |

## Core API

### Compile `evm_read` / `evm_call` / `evm_rpc`

Use the async compiler if your execution spec contains async `{ detect: ... }` ValueRefs.

```ts
import { compileEvmExecutionAsync } from '@owliabot/ais-ts-sdk';

const compiled = await compileEvmExecutionAsync(exec, ctx, {
  chain: 'eip155:1',
  params: { amount: 42n },
  detect, // optional
});

compiled.to;
compiled.data;
compiled.value;
compiled.chainId;
```
