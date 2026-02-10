/**
 * Resolver context - state container for resolution operations
 */
import type { ProtocolSpec } from '../schema/index.js';

export interface RuntimeNodeState {
  args?: Record<string, unknown>;
  outputs?: Record<string, unknown>;
  calculated?: Record<string, unknown>;
}

export interface RuntimeContext {
  /** Workflow inputs */
  inputs: Record<string, unknown>;
  /** Action/query params */
  params: Record<string, unknown>;
  /** Runtime environment (wallet, chain, time, etc.) */
  ctx: Record<string, unknown>;
  /** Query results keyed by query id */
  query: Record<string, unknown>;
  /** Deployment contracts for the selected chain */
  contracts: Record<string, unknown>;
  /** Calculated fields */
  calculated: Record<string, unknown>;
  /** Active policy fields (pack/workflow) */
  policy: Record<string, unknown>;
  /** Per-node runtime state for workflow execution */
  nodes: Record<string, RuntimeNodeState>;
}

export interface ResolverContext {
  /** Loaded protocol specs by protocol name */
  protocols: Map<string, ProtocolSpec>;
  /** Structured runtime context for refs/CEL */
  runtime: RuntimeContext;
}

/**
 * Create a fresh resolver context
 */
export function createContext(): ResolverContext {
  return {
    protocols: new Map(),
    runtime: {
      inputs: {},
      params: {},
      ctx: { capabilities: [] },
      query: {},
      contracts: {},
      calculated: {},
      policy: {},
      nodes: {},
    },
  };
}

/**
 * Get the root object for ref/CEL evaluation.
 */
export function getRuntimeRoot(ctx: ResolverContext): Record<string, unknown> {
  return {
    inputs: ctx.runtime.inputs,
    params: ctx.runtime.params,
    ctx: ctx.runtime.ctx,
    query: ctx.runtime.query,
    contracts: ctx.runtime.contracts,
    calculated: ctx.runtime.calculated,
    policy: ctx.runtime.policy,
    nodes: ctx.runtime.nodes,
  };
}

/**
 * Resolve a dot-path reference against a provided root object.
 * Returns `undefined` if any segment is missing.
 */
export function getRefFromRoot(root: Record<string, unknown>, path: string): unknown {
  return getPathValue(root, path);
}

/**
 * Resolve a dot-path reference against the runtime root.
 * Returns `undefined` if any segment is missing.
 */
export function getRef(
  ctx: ResolverContext,
  path: string,
  options?: { root?: Record<string, unknown> }
): unknown {
  return getPathValue(options?.root ?? getRuntimeRoot(ctx), path);
}

/**
 * Set a value by dot-path into the structured runtime context.
 * Missing objects are created on demand.
 */
export function setRef(
  ctx: ResolverContext,
  path: string,
  value: unknown
): void {
  const parts = path.split('.').filter(Boolean);
  const root = parts[0];
  if (
    root !== 'inputs' &&
    root !== 'params' &&
    root !== 'ctx' &&
    root !== 'query' &&
    root !== 'contracts' &&
    root !== 'calculated' &&
    root !== 'policy' &&
    root !== 'nodes'
  ) {
    throw new Error(`Unknown ref root "${root ?? ''}" for path "${path}"`);
  }
  setPathValue(ctx.runtime, parts.join('.'), value);
}

/**
 * Store query results in the context (equivalent to `setRef(ctx, "query.<id>", result)`).
 */
export function setQueryResult(
  ctx: ResolverContext,
  queryName: string,
  result: Record<string, unknown>
): void {
  ctx.runtime.query[queryName] = result;
}

/**
 * Store workflow node outputs in the context (preferred for workflow `query_ref` nodes).
 */
export function setNodeOutputs(
  ctx: ResolverContext,
  nodeId: string,
  outputs: Record<string, unknown>,
  options?: { mode?: 'set' | 'merge' }
): void {
  if (!ctx.runtime.nodes[nodeId]) ctx.runtime.nodes[nodeId] = {};
  const mode = options?.mode ?? 'set';
  if (mode === 'merge' && ctx.runtime.nodes[nodeId]!.outputs) {
    const prev = ctx.runtime.nodes[nodeId]!.outputs ?? {};
    ctx.runtime.nodes[nodeId]!.outputs = { ...prev, ...outputs };
  } else {
    ctx.runtime.nodes[nodeId]!.outputs = outputs;
  }
}

function getPathValue(root: unknown, path: string): unknown {
  if (!path) return root;
  const parts = path.split('.').filter(Boolean);
  let current: unknown = root;
  for (const part of parts) {
    if (current === null || current === undefined) return undefined;
    if (typeof current !== 'object') return undefined;
    current = (current as Record<string, unknown>)[part];
  }
  return current;
}

function setPathValue(root: unknown, path: string, value: unknown): void {
  const parts = path.split('.').filter(Boolean);
  if (parts.length === 0) return;

  let current: unknown = root;
  for (let i = 0; i < parts.length - 1; i++) {
    const part = parts[i];
    if (current === null || current === undefined) {
      throw new Error(`Cannot set ref "${path}": segment "${part}" is null/undefined`);
    }
    if (typeof current !== 'object') {
      throw new Error(`Cannot set ref "${path}": segment "${part}" is not an object`);
    }
    const record = current as Record<string, unknown>;
    if (record[part] === undefined) record[part] = {};
    current = record[part];
  }

  const last = parts[parts.length - 1]!;
  if (current === null || current === undefined || typeof current !== 'object') {
    throw new Error(`Cannot set ref "${path}": parent is not an object`);
  }
  (current as Record<string, unknown>)[last] = value;
}
