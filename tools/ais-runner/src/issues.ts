import type { RunnerWorkspaceIssue, RunnerWorkflowIssue } from './types.js';

export type StructuredIssue = {
  kind: string;
  severity: 'error' | 'warning' | 'info';
  node_id?: string;
  field_path: string;
  message: string;
  reference?: string;
  related?: { path?: string; node_id?: string; field_path?: string; reference?: string };
};

export function structuredFromWorkspaceIssues(issues: RunnerWorkspaceIssue[]): StructuredIssue[] {
  const out: StructuredIssue[] = [];
  for (const i of issues ?? []) {
    const severity = i.severity === 'warning' || i.severity === 'info' ? i.severity : 'error';
    out.push({
      kind: 'workspace_validation',
      severity,
      field_path: i.field_path ?? '(root)',
      message: i.message,
      reference: i.reference,
      related: i.related_path ? { path: i.related_path } : undefined,
    });
  }
  return out;
}

export function structuredFromWorkflowIssues(issues: RunnerWorkflowIssue[]): StructuredIssue[] {
  const out: StructuredIssue[] = [];
  for (const i of issues ?? []) {
    out.push({
      kind: 'workflow_validation',
      severity: 'error',
      node_id: i.nodeId,
      field_path: i.field ?? '(root)',
      message: i.message,
      reference: i.reference,
    });
  }
  return out;
}

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

