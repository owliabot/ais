/**
 * Workflow schema (.ais-flow.yaml)
 */
import { z } from 'zod';

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

// Inferred types
export type Workflow = z.infer<typeof WorkflowSchema>;
export type WorkflowMeta = z.infer<typeof WorkflowMetaSchema>;
export type WorkflowInput = z.infer<typeof WorkflowInputSchema>;
export type WorkflowStep = z.infer<typeof WorkflowStepSchema>;
