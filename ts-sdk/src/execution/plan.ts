/**
 * Execution Plan IR (AIS 0.0.3)
 *
 * JSON-serializable intermediate representation used to coordinate:
 * - dependency ordering (DAG)
 * - readiness checks (missing refs / detect requirements)
 * - executor scheduling (serial vs parallel)
 *
 * This layer does NOT send RPCs or broadcast transactions.
 */

import { z } from 'zod';
import type {
  ExecutionSpec,
  ExecutionBlock,
  ChainId,
  ValueRef,
  Composite,
  Workflow,
  WorkflowNode,
} from '../schema/index.js';
import {
  ChainIdSchema,
  ExecutionSpecSchema,
  ExtensionsSchema,
  ValueRefSchema,
  WorkflowSchema,
  isCoreExecutionSpec,
} from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { resolveAction, resolveQuery } from '../resolver/index.js';
import {
  evaluateValueRef,
  evaluateValueRefAsync,
  type EvaluateValueRefOptions,
  ValueRefEvalError,
} from '../resolver/index.js';
import { buildWorkflowDag, WorkflowDagError } from '../workflow/dag.js';
import { defaultExecutionTypeRegistry } from '../plugins/index.js';
import type { ExecutionTypePlugin } from '../plugins/index.js';

// ──────────────────────────────────────────────────────────────────────────────
// Plan Schema (JSON)
// ──────────────────────────────────────────────────────────────────────────────

const PlanBindingsSchema = z
  .object({
    params: z.record(ValueRefSchema).optional(),
    extensions: ExtensionsSchema,
  })
  .strict();

const PlanSourceSchema = z
  .object({
    workflow: z.object({ name: z.string(), version: z.string() }).strict().optional(),
    node_id: z.string().optional(),
    protocol: z.string().optional(),
    action: z.string().optional(),
    query: z.string().optional(),
    step_id: z.string().optional(),
    extensions: ExtensionsSchema,
  })
  .strict();

const PlanWriteSchema = z
  .object({
    path: z.string(),
    mode: z.enum(['set', 'merge']).default('set'),
    extensions: ExtensionsSchema,
  })
  .strict();

const PlanRetrySchema = z
  .object({
    interval_ms: z.number().int().positive(),
    max_attempts: z.number().int().positive().optional(),
    backoff: z.enum(['fixed']).optional(),
    extensions: ExtensionsSchema,
  })
  .strict();

export const ExecutionPlanNodeSchema = z.object({
  id: z.string(),
  chain: ChainIdSchema,
  kind: z.enum(['action_ref', 'query_ref', 'execution']),
  description: z.string().optional(),
  deps: z.array(z.string()).optional(),
  condition: ValueRefSchema.optional(),
  assert: ValueRefSchema.optional(),
  assert_message: z.string().optional(),
  until: ValueRefSchema.optional(),
  retry: PlanRetrySchema.optional(),
  timeout_ms: z.number().int().positive().optional(),
  bindings: PlanBindingsSchema.optional(),
  execution: ExecutionSpecSchema,
  writes: z.array(PlanWriteSchema).optional(),
  source: PlanSourceSchema.optional(),
  extensions: ExtensionsSchema,
}).strict();

export const ExecutionPlanSchema = z
  .object({
  schema: z.literal('ais-plan/0.0.3'),
  meta: z
    .object({
      created_at: z.string().optional(),
      name: z.string().optional(),
      description: z.string().optional(),
      extensions: ExtensionsSchema,
    })
    .strict()
    .optional(),
  nodes: z.array(ExecutionPlanNodeSchema),
  extensions: ExtensionsSchema,
})
  .strict();

export type ExecutionPlanNode = z.infer<typeof ExecutionPlanNodeSchema>;
export type ExecutionPlan = z.infer<typeof ExecutionPlanSchema>;
export type PlanWrite = z.infer<typeof PlanWriteSchema>;

export class PlanBuildError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'PlanBuildError';
  }
}

// ──────────────────────────────────────────────────────────────────────────────
// Builder: Workflow → ExecutionPlan
// ──────────────────────────────────────────────────────────────────────────────

export interface BuildWorkflowExecutionPlanOptions {
  /**
   * Default chain id if a node does not specify `nodes[].chain` and the workflow
   * does not specify `workflow.default_chain`.
   */
  default_chain?: ChainId;
}

export function buildWorkflowExecutionPlan(
  workflow: Workflow,
  ctx: ResolverContext,
  options?: BuildWorkflowExecutionPlanOptions
): ExecutionPlan {
  const parsedWorkflow = WorkflowSchema.parse(workflow);
  const nodes: ExecutionPlanNode[] = [];
  const plannerDefaultChain = options?.default_chain;

  let dag;
  try {
    dag = buildWorkflowDag(parsedWorkflow, { include_implicit_deps: true });
  } catch (err) {
    if (err instanceof WorkflowDagError) {
      throw new PlanBuildError(err.message);
    }
    throw err;
  }

  const nodesById = new Map(parsedWorkflow.nodes.map((n) => [n.id, n] as const));
  for (const id of dag.order) {
    const node = nodesById.get(id);
    if (!node) throw new PlanBuildError(`Internal error: node "${id}" not found`);
    nodes.push(
      ...buildWorkflowNodePlanNodes(
        node,
        parsedWorkflow,
        ctx,
        plannerDefaultChain,
        dag.deps_by_node_id[id]
      )
    );
  }

  return {
    schema: 'ais-plan/0.0.3',
    meta: {
      created_at: new Date().toISOString(),
      name: parsedWorkflow.meta.name,
      description: parsedWorkflow.meta.description,
    },
    nodes,
  };
}

function buildWorkflowNodePlanNodes(
  node: WorkflowNode,
  workflow: Workflow,
  ctx: ResolverContext,
  plannerDefaultChain: ChainId | undefined,
  depsOverride?: string[]
): ExecutionPlanNode[] {
  const chain = resolveNodeChain(node, workflow, plannerDefaultChain);
  const depsRaw = depsOverride ? depsOverride.slice() : node.deps?.slice();
  const deps = depsRaw && depsRaw.length > 0 ? depsRaw : undefined;
  const condition = node.condition;
  const assert = (node as any).assert as ValueRef | undefined;
  const assert_message = (node as any).assert_message as string | undefined;
  const bindings = node.args ? { params: node.args } : undefined;
  const until = node.until;
  const retry = node.retry;
  const timeout_ms = node.timeout_ms;

  if (node.type === 'action_ref') {
    if (!node.action) throw new PlanBuildError(`Workflow node "${node.id}" missing action`);
    const protoRef = String((node as any).protocol ?? '');
    const resolved = resolveAction(ctx, `${protoRef}/${node.action}`);
    if (!resolved) throw new PlanBuildError(`Action not found: ${protoRef}/${node.action}`);
    const exec = selectExecutionSpec(resolved.action.execution, chain);
    if (!(isCoreExecutionSpec(exec) && exec.type === 'composite')) {
      return [
        {
          id: node.id,
          chain,
          kind: 'action_ref',
          description: resolved.action.description,
          deps,
          condition,
          assert,
          assert_message,
          until,
          retry,
          timeout_ms,
          bindings,
          execution: exec,
          source: {
            workflow: { name: workflow.meta.name, version: workflow.meta.version },
            node_id: node.id,
            protocol: protoRef,
            action: node.action,
          },
        },
      ];
    }

    if (until || retry || timeout_ms) {
      throw new PlanBuildError(`Action node "${node.id}" cannot use until/retry/timeout_ms (only read nodes support polling)`);
    }

    return expandCompositeToPlanNodes({
      workflow,
      parent_node_id: node.id,
      parent_chain: chain,
      parent_deps: deps,
      parent_condition: condition,
      parent_assert: assert,
      parent_assert_message: assert_message,
      bindings,
      source: {
        workflow: { name: workflow.meta.name, version: workflow.meta.version },
        node_id: node.id,
        protocol: protoRef,
        action: node.action,
      },
      composite: exec,
    });
  }

  if (!node.query) throw new PlanBuildError(`Workflow node "${node.id}" missing query`);
  const protoRef = String((node as any).protocol ?? '');
  const resolved = resolveQuery(ctx, `${protoRef}/${node.query}`);
  if (!resolved) throw new PlanBuildError(`Query not found: ${protoRef}/${node.query}`);
  const exec = selectExecutionSpec(resolved.query.execution, chain);
  if (isCoreExecutionSpec(exec) && exec.type === 'composite') {
    throw new PlanBuildError(`Query node "${node.id}" cannot use composite execution`);
  }
  return [
    {
      id: node.id,
      chain,
      kind: 'query_ref',
      description: resolved.query.description,
      deps,
      condition,
      assert,
      assert_message,
      until,
      retry,
      timeout_ms,
      bindings,
      execution: exec,
      writes: [{ path: `nodes.${node.id}.outputs`, mode: 'set' }],
      source: {
        workflow: { name: workflow.meta.name, version: workflow.meta.version },
        node_id: node.id,
        protocol: protoRef,
        query: node.query,
      },
    },
  ];
}

export function selectExecutionSpec(execution: ExecutionBlock, chain: string): ExecutionSpec {
  // Match order: exact → namespace wildcard → global wildcard
  if (execution[chain]) return execution[chain] as ExecutionSpec;
  const [ns] = chain.split(':', 1);
  const nsKey = ns ? `${ns}:*` : undefined;
  if (nsKey && execution[nsKey]) return execution[nsKey] as ExecutionSpec;
  if (execution['*']) return execution['*'] as ExecutionSpec;
  throw new PlanBuildError(`No execution matches chain "${chain}"`);
}

function expandCompositeToPlanNodes(args: {
  workflow: Workflow;
  parent_node_id: string;
  parent_chain: ChainId;
  parent_deps?: string[];
  parent_condition?: ValueRef;
  parent_assert?: ValueRef;
  parent_assert_message?: string;
  bindings?: { params?: Record<string, ValueRef> };
  source: z.infer<typeof PlanSourceSchema>;
  composite: Composite;
}): ExecutionPlanNode[] {
  const steps = args.composite.steps;
  if (!steps || steps.length === 0) throw new PlanBuildError(`Composite execution for "${args.parent_node_id}" has no steps`);

  const out: ExecutionPlanNode[] = [];
  let prevId: string | null = null;

  for (let i = 0; i < steps.length; i++) {
    const step = steps[i]!;
    if ((step.execution as any)?.type === 'composite') {
      throw new PlanBuildError(
        `Nested composite is not supported (node="${args.parent_node_id}", step="${step.id}")`
      );
    }

    const isLast = i === steps.length - 1;
    const id = isLast ? args.parent_node_id : makeCompositeStepNodeId(args.parent_node_id, step.id);
    const chain = step.chain ? ChainIdSchema.parse(step.chain) : args.parent_chain;

    const deps = prevId
      ? [prevId]
      : args.parent_deps && args.parent_deps.length > 0
        ? args.parent_deps.slice()
        : undefined;
    const condition = combineConditions(args.parent_condition, step.condition, {
      parent_node_id: args.parent_node_id,
      step_id: step.id,
    });

    const stepWritesPath = `nodes.${args.parent_node_id}.outputs.steps.${step.id}`;
    const writes: PlanWrite[] = isLast
      ? [
          { path: `nodes.${args.parent_node_id}.outputs`, mode: 'merge' },
          { path: stepWritesPath, mode: 'set' },
        ]
      : [{ path: stepWritesPath, mode: 'set' }];

    out.push({
      id,
      chain,
      kind: 'execution',
      description: step.description,
      deps,
      condition,
      assert: isLast ? args.parent_assert : undefined,
      assert_message: isLast ? args.parent_assert_message : undefined,
      bindings: args.bindings,
      execution: step.execution as ExecutionSpec,
      writes,
      source: { ...args.source, step_id: step.id },
    });

    prevId = id;
  }

  return out;
}

function makeCompositeStepNodeId(parentId: string, stepId: string): string {
  return `${parentId}__${stepId}`;
}

function combineConditions(
  parent: ValueRef | undefined,
  step: ValueRef | undefined,
  where: { parent_node_id: string; step_id: string }
): ValueRef | undefined {
  if (!parent && !step) return undefined;
  if (parent && !step) return parent;
  if (!parent && step) return step;
  const a = valueRefToCelBoolean(parent!, where, 'node.condition');
  const b = valueRefToCelBoolean(step!, where, 'step.condition');
  return { cel: `((${a}) && (${b}))` };
}

function valueRefToCelBoolean(
  v: ValueRef,
  where: { parent_node_id: string; step_id: string },
  label: string
): string {
  if (!v || typeof v !== 'object') {
    throw new PlanBuildError(`Invalid ${label} for composite (node="${where.parent_node_id}", step="${where.step_id}")`);
  }
  if ('cel' in v) {
    if (typeof (v as any).cel !== 'string') throw new PlanBuildError(`${label} must be { cel: string }`);
    return (v as any).cel;
  }
  if ('ref' in v) {
    if (typeof (v as any).ref !== 'string') throw new PlanBuildError(`${label} must be { ref: string }`);
    return (v as any).ref;
  }
  if ('lit' in v) {
    const lit = (v as any).lit;
    if (typeof lit !== 'boolean') {
      throw new PlanBuildError(
        `${label} must resolve to boolean for composite AND (node="${where.parent_node_id}", step="${where.step_id}")`
      );
    }
    return lit ? 'true' : 'false';
  }
  throw new PlanBuildError(
    `${label} must be one of { cel/ref/lit<boolean> } to combine conditions (node="${where.parent_node_id}", step="${where.step_id}")`
  );
}

function resolveNodeChain(
  node: WorkflowNode,
  workflow: Workflow,
  plannerDefaultChain?: ChainId
): ChainId {
  const chain = node.chain ?? workflow.default_chain ?? plannerDefaultChain;
  if (!chain) {
    throw new PlanBuildError(
      `Workflow node "${node.id}" missing chain; set nodes[].chain or workflow.default_chain or provide planner default_chain`
    );
  }
  return ChainIdSchema.parse(chain);
}

// ──────────────────────────────────────────────────────────────────────────────
// Readiness checks (missing refs / detect / condition)
// ──────────────────────────────────────────────────────────────────────────────

export type NodeRunState = 'ready' | 'blocked' | 'skipped';

export interface NodeReadinessResult {
  state: NodeRunState;
  missing_refs: string[];
  needs_detect: boolean;
  errors: string[];
  resolved_params?: Record<string, unknown>;
}

export function getNodeReadiness(
  node: ExecutionPlanNode,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions = {}
): NodeReadinessResult {
  // 1) Condition (workflow-level) is evaluated without node params
  if (node.condition) {
    const cond = safeEval(node.condition, ctx, options);
    if (!cond.ok) {
      return {
        state: 'blocked',
        missing_refs: cond.missing_refs,
        needs_detect: cond.needs_detect,
        errors: cond.errors,
      };
    }
    if (cond.value === false) {
      return { state: 'skipped', missing_refs: [], needs_detect: false, errors: [] };
    }
    if (cond.value !== true) {
      return {
        state: 'blocked',
        missing_refs: [],
        needs_detect: false,
        errors: [`condition must evaluate to boolean, got: ${typeof cond.value}`],
      };
    }
  }

  // 2) Resolve node params (bindings)
  const resolvedParams: Record<string, unknown> = {};
  const missing: string[] = [];
  const errors: string[] = [];
  let needsDetect = false;

  if (node.bindings?.params) {
    for (const [k, v] of Object.entries(node.bindings.params)) {
      const r = safeEval(v, ctx, options);
      if (!r.ok) {
        missing.push(...r.missing_refs);
        needsDetect = needsDetect || r.needs_detect;
        errors.push(...r.errors);
      } else {
        resolvedParams[k] = r.value;
      }
    }
  }

  // 3) Execution spec readiness (evaluate ValueRefs with params override)
  const execRefs = collectExecutionValueRefs(node.execution);
  for (const v of execRefs) {
    const r = safeEval(v, ctx, { ...options, root_overrides: { params: resolvedParams } });
    if (!r.ok) {
      missing.push(...r.missing_refs);
      needsDetect = needsDetect || r.needs_detect;
      errors.push(...r.errors);
    }
  }

  if (missing.length > 0 || needsDetect || errors.length > 0) {
    return {
      state: 'blocked',
      missing_refs: uniq(missing),
      needs_detect: needsDetect,
      errors,
      resolved_params: resolvedParams,
    };
  }

  return {
    state: 'ready',
    missing_refs: [],
    needs_detect: false,
    errors: [],
    resolved_params: resolvedParams,
  };
}

export async function getNodeReadinessAsync(
  node: ExecutionPlanNode,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions = {}
): Promise<NodeReadinessResult> {
  // 1) Condition (workflow-level) is evaluated without node params
  if (node.condition) {
    const cond = await safeEvalAsync(node.condition, ctx, options);
    if (!cond.ok) {
      return {
        state: 'blocked',
        missing_refs: cond.missing_refs,
        needs_detect: cond.needs_detect,
        errors: cond.errors,
      };
    }
    if (cond.value === false) {
      return { state: 'skipped', missing_refs: [], needs_detect: false, errors: [] };
    }
    if (cond.value !== true) {
      return {
        state: 'blocked',
        missing_refs: [],
        needs_detect: false,
        errors: [`condition must evaluate to boolean, got: ${typeof cond.value}`],
      };
    }
  }

  // 2) Resolve node params (bindings)
  const resolvedParams: Record<string, unknown> = {};
  const missing: string[] = [];
  const errors: string[] = [];
  let needsDetect = false;

  if (node.bindings?.params) {
    for (const [k, v] of Object.entries(node.bindings.params)) {
      const r = await safeEvalAsync(v, ctx, options);
      if (!r.ok) {
        missing.push(...r.missing_refs);
        needsDetect = needsDetect || r.needs_detect;
        errors.push(...r.errors);
      } else {
        resolvedParams[k] = r.value;
      }
    }
  }

  // 3) Execution spec readiness (evaluate ValueRefs with params override)
  const execRefs = collectExecutionValueRefs(node.execution);
  for (const v of execRefs) {
    const r = await safeEvalAsync(v, ctx, { ...options, root_overrides: { params: resolvedParams } });
    if (!r.ok) {
      missing.push(...r.missing_refs);
      needsDetect = needsDetect || r.needs_detect;
      errors.push(...r.errors);
    }
  }

  if (missing.length > 0 || needsDetect || errors.length > 0) {
    return {
      state: 'blocked',
      missing_refs: uniq(missing),
      needs_detect: needsDetect,
      errors,
      resolved_params: resolvedParams,
    };
  }

  return {
    state: 'ready',
    missing_refs: [],
    needs_detect: false,
    errors: [],
    resolved_params: resolvedParams,
  };
}

type SafeEvalOk = { ok: true; value: unknown };
type SafeEvalErr = { ok: false; missing_refs: string[]; needs_detect: boolean; errors: string[] };
type SafeEvalResult = SafeEvalOk | SafeEvalErr;

function safeEval(value: ValueRef, ctx: ResolverContext, options: EvaluateValueRefOptions): SafeEvalResult {
  try {
    const v = evaluateValueRef(value, ctx, options);
    return { ok: true, value: v };
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      if (e.refPath) return { ok: false, missing_refs: [e.refPath], needs_detect: false, errors: [] };
      const msg = String(e.message);
      if (msg.includes('Detect kind') || msg.includes('Async detect')) {
        return { ok: false, missing_refs: [], needs_detect: true, errors: [msg] };
      }
      return { ok: false, missing_refs: [], needs_detect: false, errors: [msg] };
    }
    return { ok: false, missing_refs: [], needs_detect: false, errors: [String((e as Error).message ?? e)] };
  }
}

async function safeEvalAsync(value: ValueRef, ctx: ResolverContext, options: EvaluateValueRefOptions): Promise<SafeEvalResult> {
  try {
    const v = await evaluateValueRefAsync(value, ctx, options);
    return { ok: true, value: v };
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      if (e.refPath) return { ok: false, missing_refs: [e.refPath], needs_detect: false, errors: [] };
      const msg = String(e.message);
      if (msg.includes('Detect kind') || msg.includes('Async detect') || msg.includes('Detect provider')) {
        return { ok: false, missing_refs: [], needs_detect: true, errors: [msg] };
      }
      return { ok: false, missing_refs: [], needs_detect: false, errors: [msg] };
    }
    return { ok: false, missing_refs: [], needs_detect: false, errors: [String((e as Error).message ?? e)] };
  }
}

function uniq(xs: string[]): string[] {
  return Array.from(new Set(xs));
}

function collectExecutionValueRefs(execution: ExecutionSpec): ValueRef[] {
  if (!isCoreExecutionSpec(execution)) {
    // Plugin execution type (registry-driven). Best-effort ValueRef collection:
    // 1) plugin-provided collector, else 2) generic deep scan for ValueRef-like objects.
    const plugin = defaultExecutionTypeRegistry.get(execution.type) as ExecutionTypePlugin | null;
    if (plugin?.readinessRefsCollector) {
      try {
        return plugin.readinessRefsCollector(execution as any);
      } catch {
        // fallthrough to generic scan
      }
    }
    return collectValueRefsDeep(execution);
  }

  const out: ValueRef[] = [];

  switch (execution.type) {
    case 'evm_call':
      out.push(execution.to);
      out.push(...Object.values(execution.args));
      if (execution.value) out.push(execution.value);
      return out;
    case 'evm_read':
      out.push(execution.to);
      out.push(...Object.values(execution.args));
      return out;
    case 'evm_multiread':
      for (const c of execution.calls) {
        out.push(c.to);
        out.push(...Object.values(c.args));
      }
      return out;
    case 'evm_multicall':
      out.push(execution.to);
      if (execution.deadline) out.push(execution.deadline);
      for (const c of execution.calls) {
        out.push(...Object.values(c.args));
        if (c.condition) out.push(c.condition);
      }
      return out;
    case 'composite':
      for (const s of execution.steps) {
        if (s.condition) out.push(s.condition);
        out.push(...collectExecutionValueRefs(s.execution as ExecutionSpec));
      }
      return out;
    case 'solana_instruction':
      out.push(execution.program);
      out.push(execution.data);
      if (execution.discriminator) out.push(execution.discriminator);
      if (execution.compute_units) out.push(execution.compute_units);
      if (execution.lookup_tables) out.push(execution.lookup_tables);
      for (const a of execution.accounts) {
        out.push(a.pubkey, a.signer, a.writable);
      }
      return out;
    case 'solana_read':
      if (execution.params) out.push(execution.params);
      return out;
    case 'bitcoin_psbt':
      out.push(...Object.values(execution.mapping));
      return out;
    default:
      return out;
  }
}

function collectValueRefsDeep(value: unknown): ValueRef[] {
  const out: ValueRef[] = [];
  const seen = new Set<unknown>();

  function walk(v: unknown): void {
    if (!v || typeof v !== 'object') return;
    if (seen.has(v)) return;
    seen.add(v);

    if (isValueRefLike(v)) {
      out.push(v as ValueRef);
      return;
    }

    if (Array.isArray(v)) {
      for (const item of v) walk(item);
      return;
    }

    for (const vv of Object.values(v as Record<string, unknown>)) {
      walk(vv);
    }
  }

  walk(value);
  return out;
}

function isValueRefLike(v: unknown): v is ValueRef {
  if (!v || typeof v !== 'object') return false;
  const keys = Object.keys(v as Record<string, unknown>);
  if (keys.length !== 1) return false;
  const k = keys[0];
  return k === 'lit' || k === 'ref' || k === 'cel' || k === 'detect' || k === 'object' || k === 'array';
}
