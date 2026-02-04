/**
 * AIS Protocol SDK - Zod Schemas
 * Runtime validation for AIS documents
 */

import { z } from 'zod';

// =============================================================================
// Common Schemas
// =============================================================================

const HexAddress = z.string().regex(/^0x[a-fA-F0-9]{40}$/, 'Invalid Ethereum address');

// =============================================================================
// Protocol Spec Schemas
// =============================================================================

const ProtocolMetaSchema = z.object({
  name: z.string(),
  version: z.string(),
  chain_id: z.number().int().positive(),
  description: z.string().optional(),
  addresses: z.record(HexAddress),
});

const QueryInputSchema = z.object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
});

const QueryOutputSchema = z.object({
  name: z.string(),
  type: z.string(),
  path: z.string().optional(),
  description: z.string().optional(),
});

const QuerySchema = z.object({
  name: z.string(),
  contract: z.string(),
  method: z.string(),
  inputs: z.array(QueryInputSchema).optional(),
  outputs: z.array(QueryOutputSchema),
  description: z.string().optional(),
});

const ActionInputSchema = z.object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
  default: z.unknown().optional(),
  calculated_from: z.string().optional(),
});

const ActionOutputSchema = z.object({
  name: z.string(),
  type: z.string(),
  path: z.string().optional(),
});

const ConsistencyCheckSchema = z.object({
  condition: z.string(),
  message: z.string(),
});

const ActionSchema = z.object({
  name: z.string(),
  contract: z.string(),
  method: z.string(),
  inputs: z.array(ActionInputSchema),
  outputs: z.array(ActionOutputSchema).optional(),
  requires_queries: z.array(z.string()).optional(),
  calculated_fields: z.record(z.string()).optional(),
  consistency: z.array(ConsistencyCheckSchema).optional(),
  description: z.string().optional(),
});

const CustomTypeSchema = z.object({
  name: z.string(),
  base: z.string(),
  fields: z.record(z.string()).optional(),
  description: z.string().optional(),
});

export const ProtocolSpecSchema = z.object({
  ais_version: z.string(),
  type: z.literal('protocol'),
  protocol: ProtocolMetaSchema,
  queries: z.array(QuerySchema).optional(),
  actions: z.array(ActionSchema),
  types: z.array(CustomTypeSchema).optional(),
});

// =============================================================================
// Pack Schemas
// =============================================================================

const PackMetaSchema = z.object({
  name: z.string(),
  version: z.string(),
  description: z.string().optional(),
  maintainer: z.string().optional(),
});

const ProtocolRefSchema = z.object({
  protocol: z.string(),
  version: z.string(),
  source: z.string().optional(),
  actions: z.array(z.string()).optional(),
});

const AmountConstraintSchema = z.object({
  max_usd: z.number().positive().optional(),
  max_percentage_of_balance: z.number().min(0).max(100).optional(),
});

const SlippageConstraintSchema = z.object({
  max_bps: z.number().int().min(0).max(10000),
});

const PackConstraintsSchema = z.object({
  tokens: z.object({
    allowlist: z.array(z.string()).optional(),
    blocklist: z.array(z.string()).optional(),
  }).optional(),
  amounts: AmountConstraintSchema.optional(),
  slippage: SlippageConstraintSchema.optional(),
  require_simulation: z.boolean().optional(),
});

export const PackSchema = z.object({
  ais_version: z.string(),
  type: z.literal('pack'),
  pack: PackMetaSchema,
  protocols: z.array(ProtocolRefSchema),
  constraints: PackConstraintsSchema.optional(),
});

// =============================================================================
// Workflow Schemas
// =============================================================================

const WorkflowMetaSchema = z.object({
  name: z.string(),
  version: z.string(),
  description: z.string().optional(),
});

const WorkflowInputSchema = z.object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
  default: z.unknown().optional(),
});

const WorkflowStepSchema = z.object({
  id: z.string(),
  uses: z.string(),
  with: z.record(z.unknown()),
  outputs: z.record(z.string()).optional(),
  condition: z.string().optional(),
});

export const WorkflowSchema = z.object({
  ais_version: z.string(),
  type: z.literal('workflow'),
  workflow: WorkflowMetaSchema,
  inputs: z.array(WorkflowInputSchema),
  steps: z.array(WorkflowStepSchema),
});

// =============================================================================
// Discriminated Union
// =============================================================================

export const AISDocumentSchema = z.discriminatedUnion('type', [
  ProtocolSpecSchema,
  PackSchema,
  WorkflowSchema,
]);

// =============================================================================
// Asset Schemas
// =============================================================================

export const AssetSchema = z.object({
  chain_id: z.number().int().positive(),
  address: HexAddress,
  symbol: z.string().optional(),
  decimals: z.number().int().min(0).max(77).optional(),
});

export const TokenAmountSchema = z.object({
  asset: z.union([AssetSchema, z.string()]),
  amount: z.string(),
  human_readable: z.string().optional(),
});
