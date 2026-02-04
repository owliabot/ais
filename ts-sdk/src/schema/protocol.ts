/**
 * Protocol Spec schema (.ais.yaml)
 */
import { z } from 'zod';
import { HexAddressSchema } from './common.js';

const ProtocolMetaSchema = z.object({
  name: z.string(),
  version: z.string(),
  chain_id: z.number().int().positive(),
  description: z.string().optional(),
  addresses: z.record(HexAddressSchema),
});

const QueryInputSchema = z.object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
});

const QueryOutputSchema = z.object({
  name: z.string(),
  type: z.string(),
  path: z.string().optional(),
  description: z.string().optional(),
});

const QuerySchema = z.object({
  name: z.string(),
  contract: z.string(),
  method: z.string(),
  inputs: z.array(QueryInputSchema).optional(),
  outputs: z.array(QueryOutputSchema),
  description: z.string().optional(),
});

const ActionInputSchema = z.object({
  name: z.string(),
  type: z.string(),
  description: z.string().optional(),
  default: z.unknown().optional(),
  calculated_from: z.string().optional(),
});

const ActionOutputSchema = z.object({
  name: z.string(),
  type: z.string(),
  path: z.string().optional(),
});

const ConsistencyCheckSchema = z.object({
  condition: z.string(),
  message: z.string(),
});

const ActionSchema = z.object({
  name: z.string(),
  contract: z.string(),
  method: z.string(),
  inputs: z.array(ActionInputSchema),
  outputs: z.array(ActionOutputSchema).optional(),
  requires_queries: z.array(z.string()).optional(),
  calculated_fields: z.record(z.string()).optional(),
  consistency: z.array(ConsistencyCheckSchema).optional(),
  description: z.string().optional(),
});

const CustomTypeSchema = z.object({
  name: z.string(),
  base: z.string(),
  fields: z.record(z.string()).optional(),
  description: z.string().optional(),
});

export const ProtocolSpecSchema = z.object({
  ais_version: z.string(),
  type: z.literal('protocol'),
  protocol: ProtocolMetaSchema,
  queries: z.array(QuerySchema).optional(),
  actions: z.array(ActionSchema),
  types: z.array(CustomTypeSchema).optional(),
});

// Inferred types
export type ProtocolSpec = z.infer<typeof ProtocolSpecSchema>;
export type ProtocolMeta = z.infer<typeof ProtocolMetaSchema>;
export type Query = z.infer<typeof QuerySchema>;
export type QueryInput = z.infer<typeof QueryInputSchema>;
export type QueryOutput = z.infer<typeof QueryOutputSchema>;
export type Action = z.infer<typeof ActionSchema>;
export type ActionInput = z.infer<typeof ActionInputSchema>;
export type ActionOutput = z.infer<typeof ActionOutputSchema>;
export type ConsistencyCheck = z.infer<typeof ConsistencyCheckSchema>;
export type CustomType = z.infer<typeof CustomTypeSchema>;
