/**
 * Workflow schema (.ais-flow.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';

// ═══════════════════════════════════════════════════════════════════════════════
// Meta
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowMetaSchema = z.object({
  name: z.string(),
  version: z.string().regex(/^\d+\.\d+\.\d+$/, 'Version must be semver'),
  description: z.string().optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Pack Reference
// ═══════════════════════════════════════════════════════════════════════════════

const PackRefSchema = z.object({
  name: z.string(),
  version: z.string(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Inputs
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowInputSchema = z.object({
  type: z.string(),
  required: z.boolean().optional(),
  default: z.unknown().optional(),
  example: z.unknown().optional(),
  description: z.string().optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Calculated Override
// ═══════════════════════════════════════════════════════════════════════════════

const CalculatedOverrideSchema = z.object({
  expr: z.string(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Nodes
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowNodeSchema = z.object({
  id: z.string(),
  type: z.enum(['query_ref', 'action_ref']),
  skill: z.string().regex(
    /^[a-z0-9-]+@\d+\.\d+\.\d+$/,
    'Skill reference must be protocol@version (e.g., uniswap-v3@1.0.0)'
  ),
  query: z.string().optional(),
  action: z.string().optional(),
  args: z.record(z.unknown()).optional(),
  calculated_overrides: z.record(CalculatedOverrideSchema).optional(),
  requires_queries: z.array(z.string()).optional(),
  condition: z.string().optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Policy
// ═══════════════════════════════════════════════════════════════════════════════

const WorkflowPolicySchema = z.object({
  approvals: z.record(z.unknown()).optional(),
  hard_constraints: z.record(z.unknown()).optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Preflight
// ═══════════════════════════════════════════════════════════════════════════════

const PreflightSchema = z.object({
  simulate: z.record(z.boolean()).optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// Workflow (Top-level)
// ═══════════════════════════════════════════════════════════════════════════════

export const WorkflowSchema = z.object({
  schema: z.literal('ais-flow/1.0'),
  meta: WorkflowMetaSchema,
  requires_pack: PackRefSchema.optional(),
  inputs: z.record(WorkflowInputSchema).optional(),
  nodes: z.array(WorkflowNodeSchema),
  policy: WorkflowPolicySchema.optional(),
  preflight: PreflightSchema.optional(),
  outputs: z.record(z.string()).optional(),
});

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
