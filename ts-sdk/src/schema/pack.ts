/**
 * Pack schema (.ais-pack.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';
import { ChainIdSchema, DetectKindSchema, ExtensionsSchema } from './common.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Meta
// ═══════════════════════════════════════════════════════════════════════════════

const PackMetaSchema = z
  .object({
  name: z.string(),
  version: z.string().regex(/^\d+\.\d+\.\d+$/, 'Version must be semver'),
  description: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Includes (Protocol References)
// ═══════════════════════════════════════════════════════════════════════════════

const ProtocolIncludeSchema = z
  .object({
  protocol: z.string(),
  version: z.string(),
  source: z.enum(['registry', 'local', 'uri']).optional(),
  uri: z.string().optional(),
  chain_scope: z.array(ChainIdSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Policy
// ═══════════════════════════════════════════════════════════════════════════════

const ApprovalsSchema = z
  .object({
  auto_execute_max_risk_level: z.number().int().min(1).max(5).optional(),
  require_approval_min_risk_level: z.number().int().min(1).max(5).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const HardConstraintsDefaultsSchema = z
  .object({
  max_spend: z.string().optional(),
  max_approval: z.string().optional(),
  max_slippage_bps: z.number().int().optional(),
  allow_unlimited_approval: z.boolean().optional(),
  max_approval_multiplier: z.number().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const PolicySchema = z
  .object({
  approvals: ApprovalsSchema.optional(),
  hard_constraints_defaults: HardConstraintsDefaultsSchema.optional(),
  // Legacy fields
  risk_threshold: z.number().int().optional(),
  approval_required: z.array(z.string()).optional(),
  hard_constraints: HardConstraintsDefaultsSchema.optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Token Policy
// ═══════════════════════════════════════════════════════════════════════════════

const TokenResolutionSchema = z
  .object({
  allow_symbol_input: z.boolean().optional(),
  require_user_confirm_asset_address: z.boolean().optional(),
  require_allowlist_for_symbol_resolution: z.boolean().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const TokenAllowlistEntrySchema = z
  .object({
  chain: ChainIdSchema,
  symbol: z.string(),
  address: z.string(),
  decimals: z.number().int().optional(),
  tags: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const TokenPolicySchema = z
  .object({
  resolution: TokenResolutionSchema.optional(),
  allowlist: z.array(TokenAllowlistEntrySchema).optional(),
  // Legacy fields
  strict: z.boolean().optional(),
  permissive: z.boolean().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Providers
// ═══════════════════════════════════════════════════════════════════════════════

const QuoteProviderSchema = z
  .object({
  provider: z.string(),
  chains: z.array(ChainIdSchema).optional(),
  priority: z.number().int().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const DetectProviderSchema = z
  .object({
  kind: DetectKindSchema,
  provider: z.string(),
  chains: z.array(ChainIdSchema).optional(),
  candidates: z.array(z.unknown()).optional(),
  rule: z.string().optional(),
  priority: z.number().int().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const ProvidersSchema = z
  .object({
  quote: z
    .object({
      enabled: z.array(QuoteProviderSchema).optional(),
      extensions: ExtensionsSchema,
    })
    .strict()
    .optional(),
  detect: z
    .object({
      enabled: z.array(DetectProviderSchema).optional(),
      extensions: ExtensionsSchema,
    })
    .strict()
    .optional(),
  routing: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Plugins (Allowlists)
// ═══════════════════════════════════════════════════════════════════════════════

const ExecutionPluginAllowSchema = z
  .object({
  type: z.string().min(1),
  chains: z.array(ChainIdSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const PluginsSchema = z
  .object({
  execution: z
    .object({
      enabled: z.array(ExecutionPluginAllowSchema).optional(),
      extensions: ExtensionsSchema,
    })
    .strict()
    .optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Overrides
// ═══════════════════════════════════════════════════════════════════════════════

const ActionOverrideSchema = z
  .object({
  risk_level: z.number().int().min(1).max(5).optional(),
  risk_tags: z.array(z.string()).optional(),
  hard_constraints: HardConstraintsDefaultsSchema.optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const OverridesSchema = z
  .object({
  actions: z.record(ActionOverrideSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Pack (Top-level)
// ═══════════════════════════════════════════════════════════════════════════════

export const PackSchema = z
  .object({
  schema: z.literal('ais-pack/0.0.2'),
  
  // Meta can be inline or nested
  name: z.string().optional(),
  version: z.string().optional(),
  description: z.string().optional(),
  meta: PackMetaSchema.optional(),
  
  includes: z.array(ProtocolIncludeSchema),
  policy: PolicySchema.optional(),
  token_policy: TokenPolicySchema.optional(),
  providers: ProvidersSchema.optional(),
  plugins: PluginsSchema.optional(),
  overrides: OverridesSchema.optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Inferred Types
// ═══════════════════════════════════════════════════════════════════════════════

export type Pack = z.infer<typeof PackSchema>;
export type PackMeta = z.infer<typeof PackMetaSchema>;
export type ProtocolInclude = z.infer<typeof ProtocolIncludeSchema>;
export type Policy = z.infer<typeof PolicySchema>;
export type HardConstraintsDefaults = z.infer<typeof HardConstraintsDefaultsSchema>;
export type TokenPolicy = z.infer<typeof TokenPolicySchema>;
export type TokenAllowlistEntry = z.infer<typeof TokenAllowlistEntrySchema>;
export type Providers = z.infer<typeof ProvidersSchema>;
export type Plugins = z.infer<typeof PluginsSchema>;
export type ActionOverride = z.infer<typeof ActionOverrideSchema>;
