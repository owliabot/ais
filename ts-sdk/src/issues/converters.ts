import type { ZodError } from 'zod';
import type { PlanBuildError } from '../execution/plan.js';
import type { WorkspaceIssue } from '../validator/workspace.js';
import type { WorkflowIssue } from '../validator/workflow.js';
import type { StructuredIssue, StructuredIssueSeverity } from './structured.js';
import { zodPathToFieldPath } from './structured.js';

function normalizeSeverity(sev: string | undefined): StructuredIssueSeverity {
  if (sev === 'warning' || sev === 'info') return sev;
  return 'error';
}

export function fromWorkspaceIssues(issues: WorkspaceIssue[]): StructuredIssue[] {
  const out: StructuredIssue[] = [];
  for (const i of issues ?? []) {
    out.push({
      kind: 'workspace_validation',
      severity: normalizeSeverity((i as any).severity),
      field_path: String((i as any).field_path ?? '(root)'),
      message: String((i as any).message ?? ''),
      reference: typeof (i as any).reference === 'string' ? (i as any).reference : undefined,
      related:
        typeof (i as any).related_path === 'string'
          ? {
              path: (i as any).related_path,
            }
          : undefined,
    });
  }
  return out;
}

export function fromWorkflowIssues(issues: WorkflowIssue[]): StructuredIssue[] {
  const out: StructuredIssue[] = [];
  for (const i of issues ?? []) {
    out.push({
      kind: 'workflow_validation',
      severity: 'error',
      node_id: String((i as any).nodeId ?? ''),
      field_path: String((i as any).field ?? '(root)'),
      message: String((i as any).message ?? ''),
      reference: typeof (i as any).reference === 'string' ? (i as any).reference : undefined,
    });
  }
  return out;
}

export function fromZodError(error: ZodError, opts?: { kind?: string; severity?: StructuredIssueSeverity }): StructuredIssue[] {
  return error.issues.map((i) => ({
    kind: opts?.kind ?? 'schema_validation',
    severity: opts?.severity ?? 'error',
    field_path: zodPathToFieldPath(i.path),
    message: i.message,
    reference: i.code,
  }));
}

// PlanBuildError today is stringly; keep it stable but leave room for future richness.
export function fromPlanBuildError(error: PlanBuildError, opts?: { node_id?: string; field_path?: string }): StructuredIssue[] {
  return [
    {
      kind: 'plan_build',
      severity: 'error',
      node_id: opts?.node_id,
      field_path: opts?.field_path ?? '(root)',
      message: error.message,
    },
  ];
}

