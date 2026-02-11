/**
 * Workflow schema (.ais-flow.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';
import { ChainIdSchema, ExtensionsSchema, ValueRefSchema } from './common.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Meta
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowMetaSchema = z
  .object({
  name: z.string(),
  version: z.string().regex(/^\d+\.\d+\.\d+$/, 'Version must be semver'),
  description: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Pack Reference
// ═══════════════════════════════════════════════════════════════════════════════

const PackRefSchema = z
  .object({
  name: z.string(),
  version: z.string(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Imports
// ═══════════════════════════════════════════════════════════════════════════════

const ProtocolRefSchema = z
  .string()
  .regex(/^[a-z0-9-]+@\d+\.\d+\.\d+$/, 'Protocol reference must be protocol@version (e.g., uniswap-v3@1.0.0)');

const WorkflowImportProtocolSchema = z
  .object({
  protocol: ProtocolRefSchema,
  path: z.string().min(1),
  integrity: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

const WorkflowImportsSchema = z
  .object({
  protocols: z.array(WorkflowImportProtocolSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Inputs
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowInputSchema = z
  .object({
  type: z.string(),
  required: z.boolean().optional(),
  default: z.unknown().optional(),
  example: z.unknown().optional(),
  description: z.string().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Calculated Override
// ═══════════════════════════════════════════════════════════════════════════════

const CalculatedOverrideSchema = z
  .object({
  expr: ValueRefSchema,
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Retry / Until
// ═══════════════════════════════════════════════════════════════════════════════

const RetryPolicySchema = z
  .object({
  interval_ms: z.number().int().positive(),
  max_attempts: z.number().int().positive().optional(),
  backoff: z.enum(['fixed']).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Nodes
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowNodeSchema = z
  .object({
  id: z.string(),
  type: z.enum(['query_ref', 'action_ref']),
  chain: ChainIdSchema.optional(),
  protocol: ProtocolRefSchema,
  query: z.string().optional(),
  action: z.string().optional(),
  args: z.record(ValueRefSchema).optional(),
  calculated_overrides: z.record(CalculatedOverrideSchema).optional(),
  deps: z.array(z.string()).optional(),
  condition: ValueRefSchema.optional(),
  assert: ValueRefSchema.optional(),
  assert_message: z.string().optional(),
  until: ValueRefSchema.optional(),
  retry: RetryPolicySchema.optional(),
  timeout_ms: z.number().int().positive().optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Policy
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowPolicySchema = z
  .object({
  approvals: z.record(z.unknown()).optional(),
  hard_constraints: z.record(z.unknown()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Preflight
// ═══════════════════════════════════════════════════════════════════════════════

const PreflightSchema = z
  .object({
  simulate: z.record(z.boolean()).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Workflow (Top-level)
// ═══════════════════════════════════════════════════════════════════════════════

export const WorkflowSchema = z
  .object({
  schema: z.literal('ais-flow/0.0.3'),
  meta: WorkflowMetaSchema,
  default_chain: ChainIdSchema.optional(),
  imports: WorkflowImportsSchema.optional(),
  requires_pack: PackRefSchema.optional(),
  inputs: z.record(WorkflowInputSchema).optional(),
  nodes: z.array(WorkflowNodeSchema),
  policy: WorkflowPolicySchema.optional(),
  preflight: PreflightSchema.optional(),
  outputs: z.record(ValueRefSchema).optional(),
  extensions: ExtensionsSchema,
})
  .strict();

// ═══════════════════════════════════════════════════════════════════════════════
// Inferred Types
// ═══════════════════════════════════════════════════════════════════════════════

export type Workflow = z.infer<typeof WorkflowSchema>;
export type WorkflowMeta = z.infer<typeof WorkflowMetaSchema>;
export type WorkflowInput = z.infer<typeof WorkflowInputSchema>;
export type WorkflowNode = z.infer<typeof WorkflowNodeSchema>;
export type WorkflowPolicy = z.infer<typeof WorkflowPolicySchema>;
export type PackRef = z.infer<typeof PackRefSchema>;
export type CalculatedOverride = z.infer<typeof CalculatedOverrideSchema>;
export type RetryPolicy = z.infer<typeof RetryPolicySchema>;
export type WorkflowImports = z.infer<typeof WorkflowImportsSchema>;
export type WorkflowImportProtocol = z.infer<typeof WorkflowImportProtocolSchema>;
