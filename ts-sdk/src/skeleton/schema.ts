import { z } from 'zod';
import { ChainIdSchema, ExtensionsSchema, ValueRefSchema } from '../schema/index.js';

export const PlanSkeletonSchemaVersion = 'ais-plan-skeleton/0.0.1' as const;

const SkeletonRetrySchema = z
  .object({
    interval_ms: z.number().int().positive(),
    max_attempts: z.number().int().positive().optional(),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

const PlanSkeletonNodeBaseSchema = z
  .object({
    id: z.string().min(1),
    chain: ChainIdSchema.optional(),
    deps: z.array(z.string().min(1)).optional(),
    args: z.record(ValueRefSchema).optional(),
    condition: ValueRefSchema.optional(),
    until: ValueRefSchema.optional(),
    retry: SkeletonRetrySchema.optional(),
    timeout_ms: z.number().int().positive().optional(),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export const PlanSkeletonNodeSchema = z.discriminatedUnion('type', [
  PlanSkeletonNodeBaseSchema.extend({
    type: z.literal('action_ref'),
    protocol: z.string().min(1),
    action: z.string().min(1),
  }).strict(),
  PlanSkeletonNodeBaseSchema.extend({
    type: z.literal('query_ref'),
    protocol: z.string().min(1),
    query: z.string().min(1),
  }).strict(),
]);

export const PlanSkeletonSchema = z
  .object({
    schema: z.literal(PlanSkeletonSchemaVersion),
    meta: z
      .object({
        name: z.string().min(1).optional(),
        description: z.string().min(1).optional(),
        extensions: ExtensionsSchema.optional(),
      })
      .strict()
      .optional(),
    default_chain: ChainIdSchema.optional(),
    policy_hints: z.record(z.unknown()).optional(),
    nodes: z.array(PlanSkeletonNodeSchema).min(1),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export type PlanSkeletonNode = z.infer<typeof PlanSkeletonNodeSchema>;
export type PlanSkeleton = z.infer<typeof PlanSkeletonSchema>;

