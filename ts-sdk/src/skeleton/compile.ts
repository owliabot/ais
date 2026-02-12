import type { ChainId, Workflow } from '../schema/index.js';
import type { ExecutionPlan } from '../execution/index.js';
import { WorkflowSchema } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { validateWorkflow } from '../validator/index.js';
import { buildWorkflowExecutionPlan } from '../execution/index.js';
import { PlanSkeletonSchema, type PlanSkeleton, type PlanSkeletonNode } from './schema.js';

export type PlanSkeletonCompileIssue = {
  kind: 'skeleton_validation_error' | 'reference_error' | 'dag_error' | 'plan_build_error';
  severity: 'error' | 'warning' | 'info';
  node_id?: string;
  field_path: string;
  message: string;
  reference?: string;
  related?: Record<string, unknown>;
};

export type CompilePlanSkeletonResult =
  | { ok: true; plan: ExecutionPlan; workflow: Workflow }
  | { ok: false; issues: PlanSkeletonCompileIssue[] };

export function compilePlanSkeleton(
  input: unknown,
  ctx: ResolverContext,
  options: { default_chain?: ChainId } = {}
): CompilePlanSkeletonResult {
  const parsed = PlanSkeletonSchema.safeParse(input);
  if (!parsed.success) {
    return {
      ok: false,
      issues: parsed.error.issues.map((issue) => ({
        kind: 'skeleton_validation_error',
        severity: 'error',
        field_path: issue.path.join('.') || '<root>',
        message: issue.message,
        related: { code: issue.code },
      })),
    };
  }

  const skeleton = parsed.data as PlanSkeleton;
  const issues = validateSkeletonGraph(skeleton.nodes);
  if (issues.length > 0) return { ok: false, issues };

  const workflow = toWorkflow(skeleton, options.default_chain);
  const wfParsed = WorkflowSchema.safeParse(workflow);
  if (!wfParsed.success) {
    return {
      ok: false,
      issues: wfParsed.error.issues.map((issue) => ({
        kind: 'skeleton_validation_error',
        severity: 'error',
        field_path: `workflow.${issue.path.join('.') || '<root>'}`,
        message: issue.message,
        related: { code: issue.code },
      })),
    };
  }

  const wfValidation = validateWorkflow(workflow, ctx, { enforce_imports: false });
  if (!wfValidation.valid) {
    return {
      ok: false,
      issues: wfValidation.issues.map((i) => ({
        kind: 'reference_error',
        severity: 'error',
        node_id: i.nodeId,
        field_path: `nodes.${i.nodeId}.${i.field}`,
        message: i.message,
        reference: i.reference,
      })),
    };
  }

  try {
    const plan = buildWorkflowExecutionPlan(workflow, ctx, {
      default_chain: skeleton.default_chain ?? options.default_chain,
    });
    const ex = (plan as any).extensions ?? {};
    (plan as any).extensions = {
      ...ex,
      plan_skeleton: {
        schema: skeleton.schema,
        policy_hints: skeleton.policy_hints ?? undefined,
      },
    };
    return { ok: true, plan, workflow };
  } catch (error) {
    return {
      ok: false,
      issues: [
        {
          kind: 'plan_build_error',
          severity: 'error',
          field_path: '<plan>',
          message: (error as Error)?.message ?? String(error),
        },
      ],
    };
  }
}

function toWorkflow(skeleton: PlanSkeleton, defaultChain: ChainId | undefined): Workflow {
  const chain = skeleton.default_chain ?? defaultChain;
  return {
    schema: 'ais-flow/0.0.3',
    meta: {
      name: skeleton.meta?.name ?? 'plan-skeleton',
      version: '0.0.1',
      description: skeleton.meta?.description,
    } as any,
    default_chain: chain,
    inputs: {},
    nodes: skeleton.nodes.map((n) => nodeToWorkflowNode(n, chain)),
    extensions: skeleton.extensions ?? {},
  } as any;
}

function nodeToWorkflowNode(node: PlanSkeletonNode, defaultChain: ChainId | undefined): any {
  const base: any = {
    id: node.id,
    type: node.type,
    chain: node.chain ?? defaultChain,
    deps: node.deps,
    args: node.args,
    condition: node.condition,
    until: node.until,
    retry: node.retry,
    timeout_ms: node.timeout_ms,
    extensions: node.extensions ?? {},
  };

  if (node.type === 'action_ref') {
    return {
      ...base,
      protocol: node.protocol,
      action: node.action,
    };
  }
  return {
    ...base,
    protocol: node.protocol,
    query: (node as any).query,
  };
}

function validateSkeletonGraph(nodes: PlanSkeletonNode[]): PlanSkeletonCompileIssue[] {
  const issues: PlanSkeletonCompileIssue[] = [];
  const ids = new Set<string>();
  for (const n of nodes) {
    if (ids.has(n.id)) {
      issues.push({
        kind: 'dag_error',
        severity: 'error',
        node_id: n.id,
        field_path: `nodes.${n.id}.id`,
        message: `duplicate node id: ${n.id}`,
      });
    }
    ids.add(n.id);
  }

  for (const n of nodes) {
    for (const dep of n.deps ?? []) {
      if (!ids.has(dep)) {
        issues.push({
          kind: 'dag_error',
          severity: 'error',
          node_id: n.id,
          field_path: `nodes.${n.id}.deps`,
          message: `unknown dependency: ${dep}`,
          reference: dep,
        });
      }
    }
  }

  // Explicit deps cycle check (best-effort, workflow planner also checks implicit deps).
  const visiting = new Set<string>();
  const visited = new Set<string>();
  const byId = new Map(nodes.map((n) => [n.id, n] as const));

  const dfs = (id: string, stack: string[]) => {
    if (visited.has(id)) return;
    if (visiting.has(id)) {
      issues.push({
        kind: 'dag_error',
        severity: 'error',
        node_id: id,
        field_path: `nodes.${id}.deps`,
        message: `dependency cycle detected: ${[...stack, id].join(' -> ')}`,
      });
      return;
    }
    visiting.add(id);
    const node = byId.get(id);
    for (const dep of node?.deps ?? []) {
      dfs(dep, [...stack, id]);
    }
    visiting.delete(id);
    visited.add(id);
  };

  for (const n of nodes) dfs(n.id, []);

  return issues;
}
