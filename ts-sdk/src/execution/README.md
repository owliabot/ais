# Execution Module

Transaction building and ABI encoding for executing AIS actions on-chain. Converts high-level AIS action specifications into executable transaction calldata.

## File Structure

| File | Purpose |
|------|---------|
| `builder.ts` | Main transaction builder — resolves actions to calldata |
| `encoder.ts` | Lightweight ABI encoder (no external deps) |
| `keccak.ts` | Keccak-256 hash for function selectors |
| `multicall.ts` | Batched transaction encoding (multicall3, Universal Router) |
| `pre-authorize.ts` | Token approval flows (approve, permit, permit2) |
| `index.ts` | Re-exports all execution APIs |

## Core API

### Building Transactions

```ts
import { buildTransaction, buildQuery, buildWorkflowTransactions } from '@owliabot/ais-ts-sdk';

// Build a single action transaction
const result = await buildTransaction(
  protocol,
  'swap',
  { token_in: '0x...', token_out: '0x...', amount_in: '1000000' },
  resolverContext,
  { chain: 'eip155:1' }
);

if (result.success) {
  // result.transactions: TransactionRequest[]
  // result.resolvedParams: Record<string, unknown>
  // result.preAuthorize?: PreAuthorizeResult
}

// Build a read query (eth_call)
const queryResult = await buildQuery(
  protocol,
  'getAmountsOut',
  { amount_in: '1000000', path: ['0x...', '0x...'] },
  resolverContext,
  { chain: 'eip155:1' }
);

// Build all transactions for a workflow
const workflowTxs = await buildWorkflowTransactions(
  workflow,
  inputValues,
  resolverContext,
  { chain: 'eip155:1' }
);
```

### ABI Encoding

```ts
import {
  encodeFunctionCall,
  encodeFunctionSelector,
  encodeValue,
  buildFunctionSignature,
} from '@owliabot/ais-ts-sdk';

// Encode a function call
const calldata = encodeFunctionCall(
  'transfer(address,uint256)',
  ['address', 'uint256'],
  ['0x1234...', 1000000n]
);
// Returns: 0xa9059cbb000000...

// Just the selector
const selector = encodeFunctionSelector('transfer(address,uint256)');
// Returns: 0xa9059cbb

// Encode a single value
const encoded = encodeValue('uint256', 1000000n);
```

### Multicall Batching

```ts
import {
  buildEvmMulticall,
  encodeMulticall3,
  encodeUniversalRouter,
} from '@owliabot/ais-ts-sdk';

// Build multicall from AIS evm_multicall spec
const multicallTx = buildEvmMulticall(
  multicallSpec,
  resolverContext,
  celContext,
  evaluator,
  protocol,
  { chain: 'eip155:1', style: 'multicall3' }
);

// Encode as Multicall3 aggregate3
const data = encodeMulticall3([
  { target: '0x...', data: '0x...', allowFailure: false },
]);

// Encode as Uniswap Universal Router
const routerData = encodeUniversalRouter(commands, inputs, deadline);
```

### Pre-Authorization (Approvals)

```ts
import {
  buildPreAuthorize,
  getPreAuthorizeQueries,
  PERMIT2_ADDRESS,
} from '@owliabot/ais-ts-sdk';

// Build approval flow
const preAuth = await buildPreAuthorize(
  preAuthorizeSpec,
  resolverContext,
  celContext,
  evaluator,
  protocol,
  { chain: 'eip155:1', walletAddress: '0x...' }
);

if (preAuth.needed) {
  // preAuth.approveTx — standard approve transaction
  // preAuth.permitData — EIP-712 data for wallet to sign
  // preAuth.permit2ApproveTx — approve Permit2 contract first
}
```

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

## Execution Types (AIS-2)

The builder supports all AIS-2 execution types:

| Type | Description |
|------|-------------|
| `evm_call` | Single EVM contract call |
| `evm_read` | Read-only call (eth_call) |
| `evm_multiread` | Batched read calls |
| `evm_multicall` | Batched write calls (atomic) |
| `composite` | Multi-step with conditions |
| `solana_instruction` | Solana instruction (planned) |
| `cosmos_message` | Cosmos message (planned) |
| `bitcoin_psbt` | Bitcoin PSBT (planned) |
| `move_entry` | Move entry function (planned) |

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

- **Zero dependencies**: Encoder uses native JS (TextEncoder, BigInt) — no ethers/viem required
- **CEL integration**: Mapping values can contain CEL expressions for dynamic computation
- **Composite steps**: Each step can have conditions evaluated via CEL
- **Pre-authorize aware**: Automatically handles approve/permit/permit2 flows

## Dependencies

- `schema/` — Action, Query, ExecutionSpec type definitions
- `resolver/` — Context for variable/address resolution
- `cel/` — Expression evaluation in mapping values
