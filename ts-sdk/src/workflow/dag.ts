import type { ValueRef, Workflow, WorkflowNode } from '../schema/index.js';

export type WorkflowDagErrorKind =
  | 'duplicate_node_id'
  | 'unknown_dep'
  | 'self_dep'
  | 'cycle';

export class WorkflowDagError extends Error {
  readonly kind: WorkflowDagErrorKind;
  readonly nodeId?: string;
  readonly depId?: string;
  readonly cycle?: string[];

  constructor(
    kind: WorkflowDagErrorKind,
    message: string,
    details: { nodeId?: string; depId?: string; cycle?: string[] } = {}
  ) {
    super(message);
    this.name = 'WorkflowDagError';
    this.kind = kind;
    this.nodeId = details.nodeId;
    this.depId = details.depId;
    this.cycle = details.cycle;
  }
}

export interface WorkflowDagResult {
  order: string[];
  deps_by_node_id: Record<string, string[]>;
}

export interface BuildWorkflowDagOptions {
  include_implicit_deps?: boolean;
}

export function buildWorkflowDag(
  workflow: Workflow,
  options: BuildWorkflowDagOptions = {}
): WorkflowDagResult {
  const includeImplicit = options.include_implicit_deps ?? true;

  const nodesById = new Map<string, WorkflowNode>();
  for (const node of workflow.nodes) {
    if (nodesById.has(node.id)) {
      throw new WorkflowDagError(
        'duplicate_node_id',
        `Duplicate workflow node id "${node.id}"`,
        { nodeId: node.id }
      );
    }
    nodesById.set(node.id, node);
  }

  const nodeIds = Array.from(nodesById.keys());
  const nodeIdSet = new Set(nodeIds);

  const depsById = new Map<string, Set<string>>();
  for (const node of workflow.nodes) {
    const deps = new Set<string>();

    for (const d of node.deps ?? []) deps.add(d);
    if (includeImplicit) {
      for (const d of inferImplicitNodeDeps(node)) deps.add(d);
    }

    if (deps.has(node.id)) {
      throw new WorkflowDagError(
        'self_dep',
        `Workflow node "${node.id}" depends on itself`,
        { nodeId: node.id }
      );
    }

    for (const d of deps) {
      if (!nodeIdSet.has(d)) {
        throw new WorkflowDagError(
          'unknown_dep',
          `Workflow node "${node.id}" depends on unknown node "${d}"`,
          { nodeId: node.id, depId: d }
        );
      }
    }

    depsById.set(node.id, deps);
  }

  const order = stableTopoSort(nodeIds, depsById, workflow.nodes);

  return {
    order,
    deps_by_node_id: Object.fromEntries(
      Array.from(depsById.entries()).map(([id, deps]) => [
        id,
        Array.from(deps).sort(),
      ])
    ),
  };
}

function stableTopoSort(
  nodeIds: string[],
  depsById: Map<string, Set<string>>,
  originalNodes: WorkflowNode[]
): string[] {
  const originalIndex = new Map<string, number>();
  for (let i = 0; i < originalNodes.length; i++) originalIndex.set(originalNodes[i]!.id, i);

  const outgoing = new Map<string, Set<string>>();
  const inDegree = new Map<string, number>();
  for (const id of nodeIds) {
    outgoing.set(id, new Set());
    inDegree.set(id, 0);
  }

  for (const [id, deps] of depsById.entries()) {
    for (const dep of deps) {
      outgoing.get(dep)!.add(id);
      inDegree.set(id, (inDegree.get(id) ?? 0) + 1);
    }
  }

  const available: string[] = [];
  for (const id of nodeIds) {
    if ((inDegree.get(id) ?? 0) === 0) available.push(id);
  }

  const ordered: string[] = [];
  const sortAvailable = () =>
    available.sort((a, b) => {
      const ia = originalIndex.get(a) ?? 0;
      const ib = originalIndex.get(b) ?? 0;
      if (ia !== ib) return ia - ib;
      return a.localeCompare(b);
    });

  sortAvailable();

  while (available.length > 0) {
    const id = available.shift()!;
    ordered.push(id);

    for (const next of outgoing.get(id) ?? []) {
      const deg = (inDegree.get(next) ?? 0) - 1;
      inDegree.set(next, deg);
      if (deg === 0) {
        available.push(next);
      }
    }

    sortAvailable();
  }

  if (ordered.length !== nodeIds.length) {
    const cycle = nodeIds.filter((id) => (inDegree.get(id) ?? 0) > 0);
    throw new WorkflowDagError(
      'cycle',
      `Workflow has dependency cycle involving: ${cycle.join(', ')}`,
      { cycle }
    );
  }

  return ordered;
}

function inferImplicitNodeDeps(node: WorkflowNode): Set<string> {
  const deps = new Set<string>();

  if (node.args) {
    for (const v of Object.values(node.args)) {
      for (const d of collectNodeIdsFromValueRef(v)) deps.add(d);
    }
  }

  if (node.calculated_overrides) {
    for (const ov of Object.values(node.calculated_overrides)) {
      for (const d of collectNodeIdsFromValueRef(ov.expr)) deps.add(d);
    }
  }

  if (node.condition) {
    for (const d of collectNodeIdsFromValueRef(node.condition)) deps.add(d);
  }

  if (node.until) {
    for (const d of collectNodeIdsFromValueRef(node.until)) deps.add(d);
  }

  deps.delete(node.id);
  return deps;
}

function collectNodeIdsFromValueRef(v: ValueRef): Set<string> {
  if ('ref' in v) {
    const parts = v.ref.split('.');
    if (parts[0] === 'nodes' && parts[1]) return new Set([parts[1]]);
    return new Set();
  }

  if ('cel' in v) {
    return extractIdsFromCel(v.cel, 'nodes');
  }

  if ('object' in v) {
    const out = new Set<string>();
    for (const child of Object.values(v.object)) {
      for (const id of collectNodeIdsFromValueRef(child)) out.add(id);
    }
    return out;
  }

  if ('array' in v) {
    const out = new Set<string>();
    for (const child of v.array) {
      for (const id of collectNodeIdsFromValueRef(child)) out.add(id);
    }
    return out;
  }

  return new Set();
}

function extractIdsFromCel(cel: string, namespace: 'nodes' | 'inputs'): Set<string> {
  const out = new Set<string>();
  const re = namespace === 'nodes'
    ? /\bnodes\.([A-Za-z_][A-Za-z0-9_-]*)\b/g
    : /\binputs\.([A-Za-z_][A-Za-z0-9_-]*)\b/g;

  for (let m = re.exec(cel); m; m = re.exec(cel)) {
    if (m[1]) out.add(m[1]);
  }

  return out;
}
