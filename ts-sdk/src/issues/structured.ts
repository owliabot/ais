import { z } from 'zod';

export const StructuredIssueSeveritySchema = z.enum(['error', 'warning', 'info']);

export const StructuredIssueRelatedSchema = z
  .object({
    path: z.string().optional(),
    node_id: z.string().optional(),
    field_path: z.string().optional(),
    reference: z.string().optional(),
  })
  .strict();

// AGT104: Unified, agent-friendly issue shape.
export const StructuredIssueSchema = z
  .object({
    kind: z.string().min(1),
    severity: StructuredIssueSeveritySchema,
    node_id: z.string().optional(),
    field_path: z.string(),
    message: z.string(),
    reference: z.string().optional(),
    related: StructuredIssueRelatedSchema.optional(),
  })
  .strict();

export type StructuredIssueSeverity = z.infer<typeof StructuredIssueSeveritySchema>;
export type StructuredIssueRelated = z.infer<typeof StructuredIssueRelatedSchema>;
export type StructuredIssue = z.infer<typeof StructuredIssueSchema>;

export function zodPathToFieldPath(path: Array<string | number>): string {
  if (path.length === 0) return '(root)';
  let out = '';
  for (const seg of path) {
    if (typeof seg === 'number') {
      out += `[${seg}]`;
      continue;
    }
    if (!out) out = seg;
    else out += `.${seg}`;
  }
  return out;
}

export function issueLocator(args: { doc_path?: string; field_path?: string }): string {
  const fp = args.field_path && args.field_path.length > 0 ? args.field_path : '(root)';
  if (!args.doc_path) return fp;
  return `${args.doc_path}#${fp}`;
}

