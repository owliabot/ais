/**
 * Execution Schema - chain-specific execution specifications
 * Based on AIS-2: Execution Types
 */
import { z } from 'zod';

// ═══════════════════════════════════════════════════════════════════════════════
// Mapping Values
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Structured detection object for dynamic value resolution
 */
const DetectSchema = z.object({
  kind: z.enum(['choose_one', 'best_quote', 'best_path', 'protocol_specific']),
  provider: z.string().optional(),
  candidates: z.array(z.unknown()).optional(),
  constraints: z.record(z.unknown()).optional(),
  requires_capabilities: z.array(z.string()).optional(),
});

/**
 * Mapping value can be:
 * - string literal or reference (e.g., "params.token_in.address", "0")
 * - detect object for dynamic resolution
 * - nested object for struct params
 */
const MappingValueSchema: z.ZodType<unknown> = z.lazy(() =>
  z.union([
    z.string(),
    z.number(),
    z.object({ detect: DetectSchema }),
    z.record(MappingValueSchema),
  ])
);

const MappingSchema = z.record(MappingValueSchema);

// ═══════════════════════════════════════════════════════════════════════════════
// EVM Execution Types
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * EVM single read (eth_call)
 */
const EvmReadSchema = z.object({
  type: z.literal('evm_read'),
  contract: z.string(),
  function: z.string(),
  abi: z.string(),
  mapping: MappingSchema,
});

/**
 * EVM batched read (multicall3 or rpc batch)
 */
const EvmMultireadCallSchema = z.object({
  contract: z.string(),
  function: z.string(),
  abi: z.string(),
  mapping: MappingSchema,
  output_as: z.string(),
});

const EvmMultireadSchema = z.object({
  type: z.literal('evm_multiread'),
  method: z.enum(['multicall3', 'rpc_batch']),
  calls: z.array(EvmMultireadCallSchema),
});

/**
 * Pre-authorization methods for token approvals
 */
const PreAuthorizeSchema = z.object({
  method: z.enum(['approve', 'permit', 'permit2']),
  token: z.string(),
  spender: z.string(),
  amount: z.string(),
});

/**
 * EVM single write transaction
 */
const EvmCallSchema = z.object({
  type: z.literal('evm_call'),
  contract: z.string(),
  function: z.string(),
  abi: z.string(),
  mapping: MappingSchema,
  value: z.string().nullish(),
  pre_authorize: PreAuthorizeSchema.optional(),
});

/**
 * EVM multicall write (atomic batch)
 */
const EvmMulticallStepSchema = z.object({
  function: z.string(),
  abi: z.string(),
  mapping: MappingSchema,
  condition: z.string().optional(),
});

const EvmMulticallSchema = z.object({
  type: z.literal('evm_multicall'),
  contract: z.string(),
  calls: z.array(EvmMulticallStepSchema),
  deadline: z.string().optional(),
});

/**
 * Composite execution - multi-step with conditions
 */
const CompositeStepSchema = z.object({
  id: z.string(),
  type: z.enum(['evm_call', 'evm_read']),
  description: z.string().optional(),
  contract: z.string(),
  function: z.string(),
  abi: z.string(),
  mapping: MappingSchema,
  condition: z.string().optional(),
  deadline: z.string().optional(),
});

const CompositeSchema = z.object({
  type: z.literal('composite'),
  steps: z.array(CompositeStepSchema),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Non-EVM Execution Types (Placeholders)
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Solana instruction
 */
const SolanaAccountSchema = z.object({
  name: z.string(),
  signer: z.boolean().optional(),
  writable: z.boolean().optional(),
  source: z.string(),
  derived: z.enum(['ata', 'pda']).nullish(),
  seeds: z.array(z.string()).optional(),
  program: z.string().optional(),
});

const SolanaInstructionSchema = z.object({
  type: z.literal('solana_instruction'),
  program: z.string(),
  instruction: z.string(),
  idl: z.string().optional(),
  discriminator: z.string().optional(),
  accounts: z.array(SolanaAccountSchema),
  mapping: MappingSchema,
  compute_units: z.number().int().optional(),
  lookup_tables: z.array(z.string()).optional(),
});

/**
 * Cosmos SDK message
 */
const CosmosMessageSchema = z.object({
  type: z.literal('cosmos_message'),
  msg_type: z.string(),
  mapping: MappingSchema,
  gas_estimate: z.number().int().optional(),
  memo: z.string().optional(),
});

/**
 * Bitcoin PSBT
 */
const BitcoinOutputSchema = z.object({
  address: z.string(),
  amount: z.string(),
});

const BitcoinPsbtSchema = z.object({
  type: z.literal('bitcoin_psbt'),
  script_type: z.enum(['p2wpkh', 'p2tr', 'p2sh', 'p2wsh']),
  mapping: MappingSchema,
  op_return: z.string().optional(),
  outputs: z.array(BitcoinOutputSchema).optional(),
});

/**
 * Move entry function (Aptos/Sui)
 */
const MoveEntrySchema = z.object({
  type: z.literal('move_entry'),
  module: z.string(),
  function: z.string(),
  type_args: z.array(z.string()).optional(),
  mapping: MappingSchema,
  gas_estimate: z.number().int().optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Union & Exports
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * All execution spec types
 */
export const ExecutionSpecSchema = z.discriminatedUnion('type', [
  EvmReadSchema,
  EvmMultireadSchema,
  EvmCallSchema,
  EvmMulticallSchema,
  CompositeSchema,
  SolanaInstructionSchema,
  CosmosMessageSchema,
  BitcoinPsbtSchema,
  MoveEntrySchema,
]);

/**
 * Chain pattern → ExecutionSpec mapping
 * Patterns: "eip155:1", "eip155:*", "solana:*", "*"
 */
export const ExecutionBlockSchema = z.record(ExecutionSpecSchema);

// Inferred types
export type Detect = z.infer<typeof DetectSchema>;
export type MappingValue = z.infer<typeof MappingValueSchema>;
export type Mapping = z.infer<typeof MappingSchema>;
export type ExecutionSpec = z.infer<typeof ExecutionSpecSchema>;
export type ExecutionBlock = z.infer<typeof ExecutionBlockSchema>;
export type CompositeStep = z.infer<typeof CompositeStepSchema>;

// Individual execution types
export type EvmRead = z.infer<typeof EvmReadSchema>;
export type EvmMultiread = z.infer<typeof EvmMultireadSchema>;
export type EvmCall = z.infer<typeof EvmCallSchema>;
export type EvmMulticall = z.infer<typeof EvmMulticallSchema>;
export type Composite = z.infer<typeof CompositeSchema>;
export type SolanaInstruction = z.infer<typeof SolanaInstructionSchema>;
export type CosmosMessage = z.infer<typeof CosmosMessageSchema>;
export type BitcoinPsbt = z.infer<typeof BitcoinPsbtSchema>;
export type MoveEntry = z.infer<typeof MoveEntrySchema>;
