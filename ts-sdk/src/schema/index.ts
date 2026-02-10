/**
 * Schema module - Zod schemas with inferred TypeScript types
 */
import { z } from 'zod';

// Re-export individual schemas and types
export * from './common.js';
export * from './execution.js';
export * from './protocol.js';
export * from './pack.js';
export * from './workflow.js';
export * from './conformance.js';

// Import for union
import { ProtocolSpecSchema } from './protocol.js';
import { PackSchema } from './pack.js';
import { WorkflowSchema } from './workflow.js';

/**
 * Discriminated union of all AIS document types based on schema field
 */
export const AISDocumentSchema = z.discriminatedUnion('schema', [
  ProtocolSpecSchema,
  PackSchema,
  WorkflowSchema,
]);

export type AnyAISDocument = z.infer<typeof AISDocumentSchema>;
export type AISSchemaType = 'ais/0.0.2' | 'ais-pack/0.0.2' | 'ais-flow/0.0.2';
