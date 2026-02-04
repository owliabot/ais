/**
 * Protocol Spec schema (.ais.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';

const MetaSchema = z.object({
  protocol: z.string(),
  version: z.string(),
  name: z.string().optional(),
  homepage: z.string().optional(),
  logo: z.string().optional(),
  description: z.string().optional(),
  tags: z.array(z.string()).optional(),
  maintainer: z.string().optional(),
});

const DeploymentSchema = z.object({
  chain: z.string(), // e.g., "eip155:1"
  contracts: z.record(z.string()), // contract name → address
});

const ParamSchema = z.object({
  name: z.string(),
  type: z.string(),
  required: z.boolean().optional(),
  description: z.string().optional(),
  example: z.unknown().optional(),
  default: z.unknown().optional(),
  constraints: z.record(z.unknown()).optional(),
});

const CalculatedFieldSchema = z.object({
  type: z.string(),
  expr: z.string(),
  description: z.string().optional(),
});

const OutputSchema = z.object({
  name: z.string(),
  type: z.string(),
  path: z.string().optional(),
  description: z.string().optional(),
});

const QuerySchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
  contract: z.string(),
  method: z.string(),
  params: z.array(ParamSchema).optional(),
  outputs: z.array(OutputSchema).optional(),
  capabilities_required: z.array(z.string()).optional(),
});

const RiskSchema = z.object({
  level: z.number().int().min(1).max(5),
  tags: z.array(z.string()).optional(),
  description: z.string().optional(),
});

const ConstraintSchema = z.object({
  max_slippage_bps: z.number().int().optional(),
  min_receive_ratio: z.number().optional(),
  description: z.string().optional(),
});

const ActionSchema = z.object({
  name: z.string().optional(),
  description: z.string().optional(),
  contract: z.string(),
  method: z.string(),
  params: z.array(ParamSchema).optional(),
  calculated: z.record(CalculatedFieldSchema).optional(),
  outputs: z.array(OutputSchema).optional(),
  requires_queries: z.array(z.string()).optional(),
  risks: z.array(RiskSchema).optional(),
  constraints: z.array(ConstraintSchema).optional(),
  capabilities_required: z.array(z.string()).optional(),
});

const AssetMappingSchema = z.object({
  chain: z.string(),
  symbol: z.string(),
  address: z.string(),
  decimals: z.number().int().optional(),
});

const TestVectorSchema = z.object({
  name: z.string(),
  description: z.string().optional(),
  action: z.string(),
  inputs: z.record(z.unknown()),
  expected: z.record(z.unknown()).optional(),
});

export const ProtocolSpecSchema = z.object({
  schema: z.literal('ais/1.0'),
  meta: MetaSchema,
  deployments: z.array(DeploymentSchema),
  actions: z.record(ActionSchema),
  queries: z.record(QuerySchema).optional(),
  risks: z.array(RiskSchema).optional(),
  supported_assets: z.array(AssetMappingSchema).optional(),
  capabilities_required: z.array(z.string()).optional(),
  tests: z.array(TestVectorSchema).optional(),
});

// Inferred types
export type ProtocolSpec = z.infer<typeof ProtocolSpecSchema>;
export type Meta = z.infer<typeof MetaSchema>;
export type Deployment = z.infer<typeof DeploymentSchema>;
export type Param = z.infer<typeof ParamSchema>;
export type Query = z.infer<typeof QuerySchema>;
export type Action = z.infer<typeof ActionSchema>;
export type Risk = z.infer<typeof RiskSchema>;
export type Constraint = z.infer<typeof ConstraintSchema>;
export type CalculatedField = z.infer<typeof CalculatedFieldSchema>;
export type AssetMapping = z.infer<typeof AssetMappingSchema>;
export type TestVector = z.infer<typeof TestVectorSchema>;
