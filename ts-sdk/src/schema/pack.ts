/**
 * Pack schema (.ais-pack.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';

const HardConstraintsSchema = z.object({
  max_spend: z.string().optional(),
  max_approval: z.string().optional(),
  max_slippage_bps: z.number().int().optional(),
  allow_unlimited_approval: z.boolean().optional(),
});

const PolicySchema = z.object({
  risk_threshold: z.number().int().optional(),
  approval_required: z.array(z.string()).optional(),
  hard_constraints: HardConstraintsSchema.optional(),
});

const TokenPolicySchema = z.object({
  allowlist: z.array(z.string()).optional(),
  resolution: z.enum(['strict', 'permissive']).optional(),
});

const ProvidersSchema = z.object({
  quote: z.array(z.string()).optional(),
  routing: z.array(z.string()).optional(),
});

const SkillOverrideSchema = z.object({
  risk_tags: z.array(z.string()).optional(),
  hard_constraints: HardConstraintsSchema.optional(),
});

export const PackSchema = z.object({
  schema: z.literal('ais-pack/1.0'),
  name: z.string(),
  version: z.string(),
  description: z.string().optional(),
  includes: z.array(z.string()), // skill_id or skill_uri references
  policy: PolicySchema.optional(),
  token_policy: TokenPolicySchema.optional(),
  providers: ProvidersSchema.optional(),
  overrides: z.record(SkillOverrideSchema).optional(),
});

// Inferred types
export type Pack = z.infer<typeof PackSchema>;
export type Policy = z.infer<typeof PolicySchema>;
export type HardConstraints = z.infer<typeof HardConstraintsSchema>;
export type TokenPolicy = z.infer<typeof TokenPolicySchema>;
export type Providers = z.infer<typeof ProvidersSchema>;
export type SkillOverride = z.infer<typeof SkillOverrideSchema>;
