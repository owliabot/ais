# Schema

Zod-based schema definitions with automatic TypeScript type inference for all AIS document types.

## File Structure

- `index.ts` — Module entry point; exports all schemas and the discriminated union `AISDocumentSchema`
- `common.ts` — Shared primitives: chain IDs (CAIP-2), addresses, assets, token amounts, AIS type system
- `protocol.ts` — Protocol Spec schema (`ais/0.0.2`): meta, deployments, actions, queries, risks
- `pack.ts` — Pack schema (`ais-pack/0.0.2`): protocol bundles with policy and token settings
- `workflow.ts` — Workflow schema (`ais-flow/0.0.3`): multi-step execution graphs (ValueRef everywhere)
- `execution.ts` — Chain-specific execution specs (AIS 0.0.2): EVM JSON ABI, Solana instructions, composite steps
- `conformance.ts` — Conformance vector file schema (`ais-conformance/0.0.2`)

## Core API

### Strict schemas and `extensions`

AIS 0.0.2 core schemas are **strict**: unknown fields are rejected.

If you need to attach implementation-specific metadata, use the reserved `extensions` field (a free-form map). This is the only supported extensibility slot for core objects.

### Document Schemas

| Schema | Discriminator | Description |
|--------|---------------|-------------|
| `ProtocolSpecSchema` | `ais/0.0.2` | Single protocol definition with actions and queries |
| `PackSchema` | `ais-pack/0.0.2` | Bundle of protocol references with unified policy |
| `WorkflowSchema` | `ais-flow/0.0.3` | Multi-step execution flow referencing protocols |
| `AISDocumentSchema` | — | Discriminated union of all three |

Workflow core fields:
- `workflow.default_chain` (CAIP-2) sets the default chain for nodes.
- `nodes[].chain` (CAIP-2) overrides per node to support multi-chain workflows (e.g. EVM + Solana).
- `nodes[].protocol` uses `<protocol>@<version>` references (for example `erc20@0.0.2`).
- `workflow.imports.protocols[]` declares explicit protocol imports (`protocol` + `path`, optional `integrity`).
Workflow polling fields (engine-driven):
- `nodes[].until` (ValueRef) keeps re-running the node until the expression becomes truthy.
- `nodes[].retry` / `nodes[].timeout_ms` control polling cadence and limits.
Workflow assertion fields:
- `nodes[].assert` (ValueRef) is a post-execution single-shot check.
- `nodes[].assert_message` customizes the assertion failure message.

### Execution Types (in `execution.ts`)

| Schema | Type Field | Use Case |
|--------|------------|----------|
| `EvmReadSchema` | `evm_read` | Single `eth_call` (JSON ABI + ValueRef args) |
| `EvmMultireadSchema` | `evm_multiread` | Batched reads (multicall3 / rpc_batch) |
| `EvmCallSchema` | `evm_call` | Single write transaction (JSON ABI + ValueRef args) |
| `EvmMulticallSchema` | `evm_multicall` | Batched write calls (JSON ABI) |
| `CompositeSchema` | `composite` | Cross-chain step container (`steps[].execution`, optional `steps[].chain`) |
| `SolanaInstructionSchema` | `solana_instruction` | Solana instruction (program/accounts/data as ValueRef) |
| `SolanaReadSchema` | `solana_read` | Solana RPC read (balance/account/status queries) |
| `BitcoinPsbtSchema` | `bitcoin_psbt` | Bitcoin PSBT construction |
| `PluginExecutionSpecSchema` | *(plugin)* | Non-core execution types (validated via registry) |

JSON ABI types used by EVM execution:
- `JsonAbiFunction` — `{ type:"function", name, inputs, outputs? }`
- `JsonAbiParam` — `{ name, type, components? }` (tuple uses `type:"tuple"` with `components`)

Note: `ExecutionSpec` includes a plugin execution shape (`{ type: string, ... }`) so downstream code should either:
- handle non-core types explicitly (via plugin registry), or
- treat unknown execution types as plugin-provided.

### Common Types (in `common.ts`)

```ts
ChainIdSchema      // CAIP-2 chain ID (e.g., "eip155:1", "solana:mainnet")
HexAddressSchema   // Ethereum 0x address (40 hex chars)
ExtensionsSchema   // reserved extensibility slot (free-form map)
AssetSchema        // { chain_id, address, symbol?, decimals? }
TokenAmountSchema  // decimal string (e.g., "1.23") or "max"
AISTypeSchema      // address/bool/string/bytes/float + int/uint(8..256 step8) + bytes1..bytes32 + asset/token_amount + array<T>/tuple<...>
ValueRefSchema     // {lit|ref|cel|detect|object|array}
```

`ValueRef` is exported as an explicit TypeScript union type, so downstream code can narrow via `'lit' in v`, `'ref' in v`, etc.

## Usage Example

```ts
import { AISDocumentSchema, ProtocolSpecSchema } from './schema';
import type { ProtocolSpec, Action, ExecutionSpec } from './schema';

// Parse unknown YAML/JSON
const doc = AISDocumentSchema.parse(rawData);

if (doc.schema === 'ais/0.0.2') {
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
  ProtocolSpecSchema,  // schema: 'ais/0.0.2'
  PackSchema,          // schema: 'ais-pack/0.0.2'
  WorkflowSchema,      // schema: 'ais-flow/0.0.3'
]);
```

### Recursive Types

`ValueRefSchema` (in `common.ts`) uses `z.lazy()` for recursive nesting:

```ts
export const ValueRefSchema = z.lazy(() =>
  z.union([
    z.object({ lit: z.unknown() }).strict(),
    z.object({ ref: z.string() }).strict(),
    z.object({ cel: z.string() }).strict(),
    z.object({ detect: DetectSchema }).strict(),
    z.object({ object: z.record(ValueRefSchema) }).strict(),
    z.object({ array: z.array(ValueRefSchema) }).strict(),
  ])
);
```

## Dependencies

- **zod** — Runtime schema validation and type inference
- No internal module dependencies (this is the base layer)
