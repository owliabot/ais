/**
 * Workflow schema (.ais-flow.yaml)
 * Based on AIS-1 Core Schema
 */
import { z } from 'zod';

const WorkflowMetaSchema = z.object({
  name: z.string(),
  version: z.string(),
  description: z.string().optional(),
});

const PackRefSchema = z.object({
  name: z.string(),
  version: z.string(),
});

const WorkflowInputSchema = z.object({
  type: z.string(),
  required: z.boolean().optional(),
  default: z.unknown().optional(),
  example: z.unknown().optional(),
});

const WorkflowNodeSchema = z.object({
  id: z.string(),
  type: z.enum(['query_ref', 'action_ref']),
  skill: z.string(), // e.g., "uniswap-v3@1.0.0"
  query: z.string().optional(), // Query ID (if type=query_ref)
  action: z.string().optional(), // Action ID (if type=action_ref)
  args: z.record(z.unknown()).optional(),
  calculated_overrides: z.record(z.string()).optional(),
  requires_queries: z.array(z.string()).optional(),
  condition: z.string().optional(),
});

const WorkflowPolicySchema = z.object({
  approvals: z.record(z.unknown()).optional(),
  hard_constraints: z.record(z.unknown()).optional(),
});

const PreflightSchema = z.object({
  simulate: z.record(z.unknown()).optional(),
});

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

// Inferred types
export type Workflow = z.infer<typeof WorkflowSchema>;
export type WorkflowMeta = z.infer<typeof WorkflowMetaSchema>;
export type WorkflowInput = z.infer<typeof WorkflowInputSchema>;
export type WorkflowNode = z.infer<typeof WorkflowNodeSchema>;
export type WorkflowPolicy = z.infer<typeof WorkflowPolicySchema>;
export type PackRef = z.infer<typeof PackRefSchema>;
