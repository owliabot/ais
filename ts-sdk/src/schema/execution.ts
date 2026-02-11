/**
 * Execution Schema - chain-specific execution specifications (AIS 0.0.2)
 */
import { z } from 'zod';
import { ChainIdSchema, ExtensionsSchema, ValueRefSchema } from './common.js';
import type { ChainId, ValueRef } from './common.js';

export const CORE_EXECUTION_TYPES = [
  'evm_call',
  'evm_read',
  'evm_multiread',
  'evm_multicall',
  'solana_instruction',
  'solana_read',
  'bitcoin_psbt',
  'composite',
] as const;

export type CoreExecutionType = (typeof CORE_EXECUTION_TYPES)[number];

export function isCoreExecutionType(type: string): type is CoreExecutionType {
  return (CORE_EXECUTION_TYPES as readonly string[]).includes(type);
}

// ═══════════════════════════════════════════════════════════════════════════════
// JSON ABI (EVM)
// ═══════════════════════════════════════════════════════════════════════════════

export type JsonAbiParam = {
  name: string;
  type: string;
  components?: JsonAbiParam[];
};

export type JsonAbiFunction = {
  type: 'function';
  name: string;
  inputs: JsonAbiParam[];
  outputs?: JsonAbiParam[];
};

const JsonAbiParamSchema: z.ZodType<JsonAbiParam> = z.lazy(() =>
  z
    .object({
      name: z.string(),
      type: z.string(),
      components: z.array(JsonAbiParamSchema).optional(),
    })
    .strict()
) as z.ZodType<JsonAbiParam>;

const JsonAbiFunctionSchema: z.ZodType<JsonAbiFunction> = z.object({
  type: z.literal('function'),
  name: z.string(),
  inputs: z.array(JsonAbiParamSchema),
  outputs: z.array(JsonAbiParamSchema).optional(),
}).strict() as z.ZodType<JsonAbiFunction>;

// ═══════════════════════════════════════════════════════════════════════════════
// EVM Execution Types
// ═══════════════════════════════════════════════════════════════════════════════

const EvmCallSchema = z.object({
  type: z.literal('evm_call'),
  to: ValueRefSchema,
  abi: JsonAbiFunctionSchema,
  args: z.record(ValueRefSchema),
  value: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

const EvmReadSchema = z.object({
  type: z.literal('evm_read'),
  to: ValueRefSchema,
  abi: JsonAbiFunctionSchema,
  args: z.record(ValueRefSchema),
  extensions: ExtensionsSchema,
}).strict();

const EvmMultireadCallSchema = z.object({
  id: z.string(),
  to: ValueRefSchema,
  abi: JsonAbiFunctionSchema,
  args: z.record(ValueRefSchema),
  extensions: ExtensionsSchema,
}).strict();

const EvmMultireadSchema = z.object({
  type: z.literal('evm_multiread'),
  method: z.enum(['multicall3', 'rpc_batch']),
  calls: z.array(EvmMultireadCallSchema),
  extensions: ExtensionsSchema,
}).strict();

const EvmMulticallCallSchema = z.object({
  abi: JsonAbiFunctionSchema,
  args: z.record(ValueRefSchema),
  condition: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

const EvmMulticallSchema = z.object({
  type: z.literal('evm_multicall'),
  to: ValueRefSchema,
  calls: z.array(EvmMulticallCallSchema),
  deadline: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Solana Execution Types
// ═══════════════════════════════════════════════════════════════════════════════

const SolanaAccountSchema = z.object({
  name: z.string(),
  pubkey: ValueRefSchema,
  signer: ValueRefSchema,
  writable: ValueRefSchema,
  extensions: ExtensionsSchema,
}).strict();

const SolanaInstructionSchema = z.object({
  type: z.literal('solana_instruction'),
  program: ValueRefSchema,
  instruction: z.string(),
  discriminator: ValueRefSchema.optional(),
  accounts: z.array(SolanaAccountSchema),
  data: ValueRefSchema,
  compute_units: ValueRefSchema.optional(),
  lookup_tables: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

const SolanaReadSchema = z.object({
  type: z.literal('solana_read'),
  method: z.string().min(1),
  params: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Composite
// ═══════════════════════════════════════════════════════════════════════════════

export interface CompositeStep {
  id: string;
  description?: string;
  condition?: ValueRef;
  /**
   * Optional per-step chain override (for cross-chain composite actions).
   * If omitted, inherits the parent node/action chain.
   */
  chain?: ChainId;
  execution: ExecutionSpec;
}

const CompositeStepSchema: z.ZodType<CompositeStep> = z.lazy(() =>
  z
    .object({
    id: z
      .string()
      .regex(/^[a-zA-Z0-9_-]+$/, 'Composite step id must be [a-zA-Z0-9_-]+ (no dots/spaces)'),
    description: z.string().optional(),
    condition: ValueRefSchema.optional(),
    chain: ChainIdSchema.optional(),
    execution: ExecutionSpecSchema,
    extensions: ExtensionsSchema,
  })
    .strict()
) as z.ZodType<CompositeStep>;

const CompositeSchema = z.object({
  type: z.literal('composite'),
  steps: z
    .array(CompositeStepSchema)
    .min(1, 'Composite must have at least one step')
    .superRefine((steps, ctx) => {
      const seen = new Set<string>();
      for (let i = 0; i < steps.length; i++) {
        const id = steps[i]!.id;
        if (seen.has(id)) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: `Duplicate composite step id: "${id}"`,
            path: [i, 'id'],
          });
        }
        seen.add(id);
      }
    }),
  extensions: ExtensionsSchema,
}).strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Bitcoin Execution Types
// ═══════════════════════════════════════════════════════════════════════════════

const BitcoinPsbtSchema = z.object({
  type: z.literal('bitcoin_psbt'),
  script_type: z.enum(['p2wpkh', 'p2tr', 'p2sh', 'p2wsh']),
  mapping: z.record(ValueRefSchema),
  extensions: ExtensionsSchema,
}).strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Plugin execution types (registry-driven)
// ═══════════════════════════════════════════════════════════════════════════════

const PluginExecutionSpecSchema = z
  .object({ type: z.string().min(1) })
  .passthrough()
  .refine((v) => !CORE_EXECUTION_TYPES.includes(v.type as CoreExecutionType), {
    message: `Plugin execution type must not be one of core types: ${CORE_EXECUTION_TYPES.join(', ')}`,
  });

// ═══════════════════════════════════════════════════════════════════════════════
// Union & Exports
// ═══════════════════════════════════════════════════════════════════════════════

export type EvmRead = z.infer<typeof EvmReadSchema>;
export type EvmMultiread = z.infer<typeof EvmMultireadSchema>;
export type EvmCall = z.infer<typeof EvmCallSchema>;
export type EvmMulticall = z.infer<typeof EvmMulticallSchema>;
export type Composite = z.infer<typeof CompositeSchema>;
export type SolanaInstruction = z.infer<typeof SolanaInstructionSchema>;
export type SolanaRead = z.infer<typeof SolanaReadSchema>;
export type BitcoinPsbt = z.infer<typeof BitcoinPsbtSchema>;
export type PluginExecutionSpec = z.infer<typeof PluginExecutionSpecSchema>;

export type CoreExecutionSpec =
  | EvmRead
  | EvmMultiread
  | EvmCall
  | EvmMulticall
  | Composite
  | SolanaInstruction
  | SolanaRead
  | BitcoinPsbt;

export type ExecutionSpec = CoreExecutionSpec | PluginExecutionSpec;

export function isCoreExecutionSpec(execution: ExecutionSpec): execution is CoreExecutionSpec {
  return isCoreExecutionType(execution.type);
}

export const ExecutionSpecSchema: z.ZodType<ExecutionSpec> = z.lazy(() =>
  z.union([
    EvmReadSchema,
    EvmMultireadSchema,
    EvmCallSchema,
    EvmMulticallSchema,
    CompositeSchema,
    SolanaInstructionSchema,
    SolanaReadSchema,
    BitcoinPsbtSchema,
    PluginExecutionSpecSchema,
  ])
) as z.ZodType<ExecutionSpec>;

/**
 * Chain pattern → ExecutionSpec mapping
 * Patterns:
 * - exact CAIP-2 chain id: "eip155:1"
 * - namespace wildcard: "eip155:*"
 * - global wildcard: "*"
 */
const NamespaceWildcardChainPatternSchema = z.string().regex(
  /^(eip155|solana|cosmos|bip122|aptos|sui):\*$/,
  'Invalid chain pattern (expected "<namespace>:*" or exact CAIP-2 chain id or "*")'
);

export const ChainPatternSchema = z.union([
  z.literal('*'),
  ChainIdSchema,
  NamespaceWildcardChainPatternSchema,
]);

export const ExecutionBlockSchema = z.record(ChainPatternSchema, ExecutionSpecSchema);

export type ExecutionBlock = z.infer<typeof ExecutionBlockSchema>;
