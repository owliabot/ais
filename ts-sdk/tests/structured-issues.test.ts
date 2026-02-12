import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  StructuredIssueSchema,
  fromZodError,
  fromWorkflowIssues,
  fromWorkspaceIssues,
} from '../src/index.js';

describe('AGT104 structured issues', () => {
  it('converts ZodError into StructuredIssue[]', () => {
    const S = z.object({ a: z.number() }).strict();
    const r = S.safeParse({ a: 'x' });
    expect(r.success).toBe(false);
    const issues = fromZodError(r.error);
    expect(issues.length).toBeGreaterThan(0);
    expect(StructuredIssueSchema.safeParse(issues[0]).success).toBe(true);
  });

  it('converts WorkflowIssue', () => {
    const issues = fromWorkflowIssues([{ nodeId: 'n1', field: 'chain', message: 'Missing chain' }]);
    expect(StructuredIssueSchema.safeParse(issues[0]).success).toBe(true);
    expect(issues[0].node_id).toBe('n1');
  });

  it('converts WorkspaceIssue', () => {
    const issues = fromWorkspaceIssues([{ path: '/ws/x.yaml', severity: 'error', message: 'bad', field_path: 'meta' }]);
    expect(StructuredIssueSchema.safeParse(issues[0]).success).toBe(true);
    expect(issues[0].field_path).toBe('meta');
  });
});

