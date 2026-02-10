/**
 * Protocol Spec schema (.ais.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';
import { AISTypeSchema, ChainIdSchema, ExtensionsSchema, ValueRefSchema } from './common.js';
import { ExecutionBlockSchema } from './execution.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Meta
// ═══════════════════════════════════════════════════════════════════════════════

const MetaSchema = z
  .object({
  protocol: z.string().regex(/^[a-z0-9-]+$/, 'Protocol ID must be kebab-case'),
  version: z.string().regex(/^\d+\.\d+\.\d+$/, 'Version must be semver'),
  name: z.string().optional(),
  homepage: z.string().url().optional(),
  logo: z.string().optional(),
  description: z.string().optional(),
  tags: z.array(z.string()).optional(),
  maintainer: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Deployments
// ═══════════════════════════════════════════════════════════════════════════════

const DeploymentSchema = z
  .object({
  chain: ChainIdSchema,
  contracts: z.record(z.string()),
  rpc_hints: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Parameters
// ═══════════════════════════════════════════════════════════════════════════════

const ParamConstraintsSchema = z
  .object({
  min: z.number().optional(),
  max: z.number().optional(),
  enum: z.array(z.unknown()).optional(),
  pattern: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const ParamSchema = z
  .object({
  name: z.string(),
  type: AISTypeSchema,
  description: z.string(),
  required: z.boolean().optional(),
  default: z.unknown().optional(),
  example: z.unknown().optional(),
  constraints: ParamConstraintsSchema.optional(),
  // For token_amount type
  asset_ref: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Calculated Fields
// ═══════════════════════════════════════════════════════════════════════════════

const CalculatedFieldSchema = z
  .object({
  expr: ValueRefSchema,
  inputs: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const CalculatedFieldsSchema = z.record(CalculatedFieldSchema);

// ═══════════════════════════════════════════════════════════════════════════════
// Returns
// ═══════════════════════════════════════════════════════════════════════════════

const ReturnFieldSchema = z
  .object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Hard Constraints
// ═══════════════════════════════════════════════════════════════════════════════

const HardConstraintsSchema = z
  .object({
  max_slippage_bps: ValueRefSchema.optional(),
  max_spend: ValueRefSchema.optional(),
  max_approval: ValueRefSchema.optional(),
  allow_unlimited_approval: ValueRefSchema.optional(),
  max_price_impact_bps: ValueRefSchema.optional(),
  min_health_factor_after: ValueRefSchema.optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Risk Tags (Action-level)
// ═══════════════════════════════════════════════════════════════════════════════

const RiskTagSchema = z.string();

// ═══════════════════════════════════════════════════════════════════════════════
// Query
// ═══════════════════════════════════════════════════════════════════════════════

const ConsistencySchema = z
  .object({
  block_tag: z.union([
    z.enum(['latest', 'safe', 'finalized']),
    z.number().int(),
  ]).optional(),
  require_same_block: z.boolean().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const QuerySchema = z
  .object({
  description: z.string(),
  params: z.array(ParamSchema).optional(),
  returns: z.array(ReturnFieldSchema).optional(),
  cache_ttl: z.number().int().optional(),
  consistency: ConsistencySchema.optional(),
  execution: ExecutionBlockSchema,
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Action
// ═══════════════════════════════════════════════════════════════════════════════

const ActionSchema = z
  .object({
  description: z.string(),
  risk_level: z.number().int().min(1).max(5),
  risk_tags: z.array(RiskTagSchema).optional(),
  
  params: z.array(ParamSchema).optional(),
  returns: z.array(ReturnFieldSchema).optional(),
  
  requires_queries: z.array(z.string()).optional(),
  hard_constraints: HardConstraintsSchema.optional(),
  calculated_fields: CalculatedFieldsSchema.optional(),
  
  execution: ExecutionBlockSchema,
  
  pre_conditions: z.array(z.string()).optional(),
  side_effects: z.array(z.string()).optional(),
  capabilities_required: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Protocol-level Risks
// ═══════════════════════════════════════════════════════════════════════════════

const ProtocolRiskSchema = z
  .object({
  level: z.enum(['info', 'warning', 'critical']),
  text: z.string(),
  applies_to: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Supported Assets (Multi-chain)
// ═══════════════════════════════════════════════════════════════════════════════

const AssetMappingSchema = z
  .object({
  symbol: z.string(),
  name: z.string().optional(),
  decimals: z.record(z.number().int()),
  addresses: z.record(z.string()),
  coingecko_id: z.string().optional(),
  tags: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Test Vectors
// ═══════════════════════════════════════════════════════════════════════════════

const TestVectorExpectSchema = z
  .object({
  calculated: z.record(z.unknown()).optional(),
  execution_type: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const TestVectorSchema = z
  .object({
  name: z.string(),
  action: z.string(),
  params: z.record(z.unknown()),
  expect: TestVectorExpectSchema.optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Protocol Spec (Top-level)
// ═══════════════════════════════════════════════════════════════════════════════

export const ProtocolSpecSchema = z
  .object({
  schema: z.literal('ais/0.0.2'),
  meta: MetaSchema,
  deployments: z.array(DeploymentSchema),
  actions: z.record(ActionSchema),
  queries: z.record(QuerySchema).optional(),
  risks: z.array(ProtocolRiskSchema).optional(),
  supported_assets: z.array(AssetMappingSchema).optional(),
  capabilities_required: z.array(z.string()).optional(),
  tests: z.array(TestVectorSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Inferred Types
// ═══════════════════════════════════════════════════════════════════════════════

export type ProtocolSpec = z.infer<typeof ProtocolSpecSchema>;
export type Meta = z.infer<typeof MetaSchema>;
export type Deployment = z.infer<typeof DeploymentSchema>;
export type Param = z.infer<typeof ParamSchema>;
export type ParamConstraints = z.infer<typeof ParamConstraintsSchema>;
export type Query = z.infer<typeof QuerySchema>;
export type Action = z.infer<typeof ActionSchema>;
export type ProtocolRisk = z.infer<typeof ProtocolRiskSchema>;
export type HardConstraints = z.infer<typeof HardConstraintsSchema>;
export type CalculatedField = z.infer<typeof CalculatedFieldSchema>;
export type CalculatedFields = z.infer<typeof CalculatedFieldsSchema>;
export type AssetMapping = z.infer<typeof AssetMappingSchema>;
export type TestVector = z.infer<typeof TestVectorSchema>;
export type ReturnField = z.infer<typeof ReturnFieldSchema>;
export type Consistency = z.infer<typeof ConsistencySchema>;
export type RiskTag = z.infer<typeof RiskTagSchema>;
