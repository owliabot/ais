# Execution Module (AIS 0.0.3 plan/workflow)

Transaction building and ABI encoding for executing AIS actions on-chain.

The execution layer compiles AIS `execution` specs into chain-SDK requests (EVM calldata / Solana instructions) and provides a JSON-serializable `ExecutionPlan` IR for orchestration.

## File Structure

| File | Purpose |
|------|---------|
| `builder.ts` | Legacy builder APIs (not recommended; prefer `ExecutionPlan`) |
| `plan.ts` | JSON-serializable ExecutionPlan IR (DAG + readiness) |
| `evm/` | EVM compilation + ABI encode/decode + keccak (`evm_read` / `evm_call`) |
| `index.ts` | Re-exports all execution APIs |
| `solana/` | Solana instruction planning helpers |

## Core API

### Execution Plan IR (JSON) — Recommended entry point

`ExecutionPlan` (`ais-plan/0.0.3`) is a JSON-serializable DAG IR for coordinating reads/writes and their dependencies. Plan nodes are emitted in a stable topological order based on `deps` plus dependencies inferred from `ValueRef` references to `nodes.*`.

Composite:
- If an `action_ref` resolves to `execution.type: composite`, the planner expands it into multiple plan nodes (`kind: "execution"`), one per step, with sequential deps.
- Steps MAY override the chain via `steps[].chain` (cross-chain composite actions).
- Step outputs are written under `nodes.<parent>.outputs.steps.<step_id>`; the last step also merges into `nodes.<parent>.outputs` for convenience.

Chain selection:
- Each plan node has a concrete `chain` (CAIP-2), derived from `nodes[].chain` or `workflow.default_chain`.
- Protocol `execution` blocks MAY use wildcards like `eip155:*`; the planner selects the best match using the node's concrete chain.
Polling:
- Plan nodes may carry `until` / `retry` / `timeout_ms` (copied from workflow nodes).
- The engine can use these fields to implement “post-check until satisfied” loops (e.g. wait for bridge arrival).
Assertions:
- Plan nodes may carry `assert` / `assert_message` (copied from workflow nodes).
- `assert` is a post-execution single-shot check, distinct from polling `until`.

```ts
import {
  buildWorkflowExecutionPlan,
  getNodeReadiness,
  getNodeReadinessAsync,
  ExecutionPlanSchema,
} from '@owliabot/ais-ts-sdk';

const plan = buildWorkflowExecutionPlan(workflow, resolverContext);

// Serialize / checkpoint
const json = JSON.stringify(plan);
const restored = ExecutionPlanSchema.parse(JSON.parse(json));
// Note: ExecutionPlan schemas are strict; attach any extra metadata under `extensions`.

// Before executing a node, check readiness (missing refs / detect requirements)
for (const node of restored.nodes) {
  const r = getNodeReadiness(node, resolverContext);
  if (r.state === 'blocked') {
    // ask solver to patch runtime with r.missing_refs, or handle detect
  }
}
```

If a workflow uses async `{ detect: ... }` resolution (e.g. needs network IO for routing/quotes), use `getNodeReadinessAsync(node, ctx, { detect })` and compile via the `*Async` compilers.

### ABI Encoding (EVM)

```ts
import { encodeJsonAbiFunctionCall, decodeJsonAbiFunctionResult } from '@owliabot/ais-ts-sdk';

const abi = {
  type: 'function',
  name: 'q',
  inputs: [{ name: 'x', type: 'uint256' }],
  outputs: [{ name: 'y', type: 'uint256' }],
} as const;

const data = encodeJsonAbiFunctionCall(abi, { x: 42n });
const out = decodeJsonAbiFunctionResult(abi, '0x' + '00'.repeat(32));
```

### Multicall and Pre-Authorization

Multicall batching and token approval helpers will be reintroduced after the 0.0.2 execution refactor is complete.

## Types

### TransactionRequest

```ts
interface TransactionRequest {
  to: string;          // Target contract address
  data: string;        // Encoded calldata (0x...)
  value: bigint;       // ETH value in wei
  chainId: number;     // Numeric chain ID
  stepId?: string;     // Step identifier (for composite)
  stepDescription?: string;
}
```

### BuildOptions

```ts
interface BuildOptions {
  chain: string;              // CAIP-2 chain ID (e.g., "eip155:1")
  contractAddress?: string;   // Override target address
  value?: bigint;             // ETH value to send
  preAuthorize?: PreAuthorizeContext;  // Wallet info for approvals
  multicallStyle?: 'standard' | 'multicall3' | 'universal_router';
}
```

### PreAuthorizeResult

```ts
interface PreAuthorizeResult {
  needed: boolean;                    // Is authorization required?
  approveTx?: TransactionRequest;     // Standard ERC20 approve
  permitData?: PermitData;            // EIP-712 signature data
  permit2ApproveTx?: TransactionRequest; // Permit2 initial approval
}
```

## Execution Types (AIS 0.0.2)

The schema layer defines the AIS 0.0.2 execution types (see `src/schema/execution.ts`). The builder implementation is currently in progress.

| Type | Description |
|------|-------------|
| `evm_call` | Single EVM contract call |
| `evm_read` | Read-only call (eth_call) |
| `evm_multiread` | Batched read calls |
| `evm_multicall` | Batched write calls (atomic) |
| `composite` | Multi-step with conditions |
| `solana_instruction` | Solana instruction (compiler + RPC executor) |
| `solana_read` | Solana RPC read (balance/account/status queries) |
| `bitcoin_psbt` | Bitcoin PSBT construction (core) |
| *(plugin)* | All non-core execution types (registry-driven) |

Built-in plugin types:
- `evm_rpc` (read-only JSON-RPC calls; executor-enforced allowlist)

## Chain Pattern Matching

Execution blocks use chain patterns to target specific networks:

```yaml
execution:
  eip155:1:      # Exact: Ethereum mainnet
    type: evm_call
    ...
  eip155:*:      # Wildcard: All EVM chains
    type: evm_call
    ...
  *:             # Global fallback
    type: evm_call
    ...
```

The builder matches in order: exact → namespace wildcard → global fallback.

## Implementation Notes

- **Uses standard chain SDKs**: EVM ABI uses `ethers`; Solana uses `@solana/web3.js` / `@solana/spl-token`
- **Engine handles network IO**: execution module compiles requests; IO happens in `engine` executors
- **CEL integration**: Mapping values can contain CEL expressions for dynamic computation
- **Composite steps**: Each step can have conditions evaluated via CEL
- **Pre-authorize aware**: Automatically handles approve/permit/permit2 flows
- **Solana extensibility**: for non-standard Solana program encodings, use `solana.SolanaInstructionCompilerRegistry` to register `(programId, instruction)` compilers.

## Dependencies

- `schema/` — Action, Query, ExecutionSpec type definitions
- `resolver/` — Context for variable/address resolution
- `cel/` — Expression evaluation in mapping values
