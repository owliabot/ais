# Schema

Zod-based schema definitions with automatic TypeScript type inference for all AIS document types.

## File Structure

- `index.ts` — Module entry point; exports all schemas and the discriminated union `AISDocumentSchema`
- `common.ts` — Shared primitives: chain IDs (CAIP-2), addresses, assets, token amounts, AIS type system
- `protocol.ts` — Protocol Spec schema (`ais/1.0`): meta, deployments, actions, queries, risks
- `pack.ts` — Pack schema (`ais-pack/1.0`): skill bundles with policy and token settings
- `workflow.ts` — Workflow schema (`ais-flow/1.0`): multi-step execution graphs
- `execution.ts` — Chain-specific execution specs: EVM, Solana, Cosmos, Bitcoin, Move

## Core API

### Document Schemas

| Schema | Discriminator | Description |
|--------|---------------|-------------|
| `ProtocolSpecSchema` | `ais/1.0` | Single protocol definition with actions and queries |
| `PackSchema` | `ais-pack/1.0` | Bundle of protocol skills with unified policy |
| `WorkflowSchema` | `ais-flow/1.0` | Multi-step execution flow referencing pack skills |
| `AISDocumentSchema` | — | Discriminated union of all three |

### Execution Types (in `execution.ts`)

| Schema | Type Field | Use Case |
|--------|------------|----------|
| `EvmReadSchema` | `evm_read` | Single `eth_call` |
| `EvmMultireadSchema` | `evm_multiread` | Batched reads (multicall3 / rpc_batch) |
| `EvmCallSchema` | `evm_call` | Single write transaction |
| `EvmMulticallSchema` | `evm_multicall` | Atomic multi-step via router |
| `CompositeSchema` | `composite` | Conditional multi-step execution |
| `SolanaInstructionSchema` | `solana_instruction` | Solana program instruction |
| `CosmosMessageSchema` | `cosmos_message` | Cosmos SDK message |
| `BitcoinPsbtSchema` | `bitcoin_psbt` | Bitcoin PSBT construction |
| `MoveEntrySchema` | `move_entry` | Aptos/Sui Move function |

### Common Types (in `common.ts`)

```ts
ChainIdSchema      // CAIP-2 chain ID (e.g., "eip155:1", "solana:mainnet")
HexAddressSchema   // Ethereum 0x address (40 hex chars)
AssetSchema        // { chain_id, address, symbol?, decimals? }
TokenAmountSchema  // { asset, amount, human_readable? }
AISTypeSchema      // Type enum: address, bool, uint256, asset, token_amount, etc.
```

## Usage Example

```ts
import { AISDocumentSchema, ProtocolSpecSchema } from './schema';
import type { ProtocolSpec, Action, ExecutionSpec } from './schema';

// Parse unknown YAML/JSON
const doc = AISDocumentSchema.parse(rawData);

if (doc.schema === 'ais/1.0') {
  // TypeScript knows this is ProtocolSpec
  console.log(doc.meta.protocol);
}

// Validate specific document type
const protocol: ProtocolSpec = ProtocolSpecSchema.parse(yaml);

// Access typed fields
const swapAction: Action = protocol.actions['swap'];
const exec: ExecutionSpec = swapAction.execution['eip155:*'];
```

## Implementation Details

### Type Inference Pattern

All schemas use Zod's type inference (`z.infer<>`) to derive TypeScript types. This ensures runtime validation and static types stay in sync:

```ts
export const AssetSchema = z.object({
  chain_id: ChainIdSchema,
  address: AddressSchema,
  symbol: z.string().optional(),
  decimals: z.number().int().optional(),
});

export type Asset = z.infer<typeof AssetSchema>;  // ← Auto-generated type
```

### Discriminated Unions

Document types are distinguished by the `schema` field using Zod's `discriminatedUnion`:

```ts
const AISDocumentSchema = z.discriminatedUnion('schema', [
  ProtocolSpecSchema,  // schema: 'ais/1.0'
  PackSchema,          // schema: 'ais-pack/1.0'
  WorkflowSchema,      // schema: 'ais-flow/1.0'
]);
```

### Recursive Types

`MappingValueSchema` (in `execution.ts`) uses `z.lazy()` for recursive nesting:

```ts
const MappingValueSchema = z.lazy(() =>
  z.union([
    z.string(),
    z.number(),
    z.object({ detect: DetectSchema }),
    z.record(MappingValueSchema),  // ← recursive
  ])
);
```

## Dependencies

- **zod** — Runtime schema validation and type inference
- No internal module dependencies (this is the base layer)
