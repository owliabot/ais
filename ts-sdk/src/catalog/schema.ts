import { z } from 'zod';
import { ExtensionsSchema } from '../schema/common.js';

export const CatalogSchemaVersion = 'ais-catalog/0.0.1' as const;

export const CardParamSchema = z
  .object({
    name: z.string().min(1),
    type: z.string().min(1),
    required: z.boolean().optional(),
    asset_ref: z.string().min(1).optional(),
  })
  .strict();

export const CardReturnSchema = z
  .object({
    name: z.string().min(1),
    type: z.string().min(1),
  })
  .strict();

export const ActionCardSchema = z
  .object({
    ref: z.string().min(1),
    protocol: z.string().min(1),
    version: z.string().min(1),
    id: z.string().min(1),
    description: z.string().min(1).optional(),
    risk_level: z.number().int().min(1).max(5),
    risk_tags: z.array(z.string().min(1)).optional(),
    params: z.array(CardParamSchema).optional(),
    returns: z.array(CardReturnSchema).optional(),
    requires_queries: z.array(z.string().min(1)).optional(),
    capabilities_required: z.array(z.string().min(1)).optional(),
    execution_types: z.array(z.string().min(1)),
    execution_chains: z.array(z.string().min(1)),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export const QueryCardSchema = z
  .object({
    ref: z.string().min(1),
    protocol: z.string().min(1),
    version: z.string().min(1),
    id: z.string().min(1),
    description: z.string().min(1).optional(),
    params: z.array(CardParamSchema).optional(),
    returns: z.array(CardReturnSchema).optional(),
    capabilities_required: z.array(z.string().min(1)).optional(),
    execution_types: z.array(z.string().min(1)),
    execution_chains: z.array(z.string().min(1)),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export const PackIncludeCardSchema = z
  .object({
    protocol: z.string().min(1),
    version: z.string().min(1),
    chain_scope: z.array(z.string().min(1)).optional(),
  })
  .strict();

export const PackCardSchema = z
  .object({
    name: z.string().min(1),
    version: z.string().min(1),
    description: z.string().min(1).optional(),
    includes: z.array(PackIncludeCardSchema),
    policy: z.record(z.unknown()).optional(),
    token_policy: z.record(z.unknown()).optional(),
    providers: z.record(z.unknown()).optional(),
    plugins: z.record(z.unknown()).optional(),
    overrides: z.record(z.unknown()).optional(),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export const CatalogDocumentEntrySchema = z
  .object({
    path: z.string().min(1),
    kind: z.enum(['protocol', 'pack', 'workflow', 'error']),
    id: z.string().min(1).optional(),
    hash: z.string().min(1),
  })
  .strict();

export const CatalogSchema = z
  .object({
    schema: z.literal(CatalogSchemaVersion),
    created_at: z.string().min(1),
    hash: z.string().min(1),
    documents: z.array(CatalogDocumentEntrySchema).optional(),
    actions: z.array(ActionCardSchema),
    queries: z.array(QueryCardSchema),
    packs: z.array(PackCardSchema),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export type ActionCard = z.infer<typeof ActionCardSchema>;
export type QueryCard = z.infer<typeof QueryCardSchema>;
export type PackCard = z.infer<typeof PackCardSchema>;
export type CatalogDocumentEntry = z.infer<typeof CatalogDocumentEntrySchema>;
export type Catalog = z.infer<typeof CatalogSchema>;

