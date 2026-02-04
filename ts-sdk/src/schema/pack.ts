/**
 * Pack schema (.ais-pack.yaml)
 */
import { z } from 'zod';

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
  tokens: z
    .object({
      allowlist: z.array(z.string()).optional(),
      blocklist: z.array(z.string()).optional(),
    })
    .optional(),
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

// Inferred types
export type Pack = z.infer<typeof PackSchema>;
export type PackMeta = z.infer<typeof PackMetaSchema>;
export type ProtocolRef = z.infer<typeof ProtocolRefSchema>;
export type PackConstraints = z.infer<typeof PackConstraintsSchema>;
export type AmountConstraint = z.infer<typeof AmountConstraintSchema>;
export type SlippageConstraint = z.infer<typeof SlippageConstraintSchema>;
