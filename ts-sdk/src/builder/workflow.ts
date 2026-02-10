/**
 * Workflow Builder - DSL for building workflows programmatically
 */

import { BaseBuilder } from './base.js';
import {
  WorkflowSchema,
  type Workflow,
  type WorkflowNode,
  type WorkflowInput,
  type CalculatedOverride,
  type ValueRef,
} from '../schema/index.js';
import type { ZodSchema } from 'zod';

interface NodeDef {
  type: 'action_ref' | 'query_ref';
  chain?: string;
  skill: string;
  action?: string;
  query?: string;
  args?: Record<string, unknown>;
  calculated_overrides?: Record<string, CalculatedOverride>;
  condition?: unknown;
  until?: unknown;
  retry?: { interval_ms: number; max_attempts?: number; backoff?: 'fixed' };
  timeout_ms?: number;
  requires?: string[];
}

export class WorkflowBuilder extends BaseBuilder<Workflow> {
  protected schema: ZodSchema<Workflow> = WorkflowSchema;

  private _meta: Workflow['meta'];
  private _inputs: Record<string, WorkflowInput> = {};
  private _nodes: WorkflowNode[] = [];
  private _outputs: Record<string, ValueRef> = {};
  private _requiresPack?: { name: string; version: string };
  private _policy?: Workflow['policy'];
  private _preflight?: Workflow['preflight'];
  private _defaultChain?: Workflow['default_chain'];

  constructor(name: string, version: string) {
    super();
    this._meta = { name, version };
  }

  /** Set workflow description */
  description(desc: string): this {
    this._meta.description = desc;
    return this;
  }

  /** Set workflow default chain (CAIP-2) */
  defaultChain(chain: Workflow['default_chain']): this {
    this._defaultChain = chain;
    return this;
  }

  /** Add an input parameter */
  input(
    name: string,
    type: string,
    options?: Omit<WorkflowInput, 'type'>
  ): this {
    this._inputs[name] = {
      type,
      required: options?.required ?? true,
      ...options,
    };
    return this;
  }

  /** Add a required input parameter */
  requiredInput(
    name: string,
    type: string,
    options?: Omit<WorkflowInput, 'type' | 'required'>
  ): this {
    this._inputs[name] = { type, required: true, ...options };
    return this;
  }

  /** Add an optional input with default value */
  optionalInput(name: string, type: string, defaultValue: unknown): this {
    this._inputs[name] = { type, required: false, default: defaultValue };
    return this;
  }

  /** Add a node (action or query) */
  node(id: string, def: NodeDef): this {
    const args = def.args ? mapRecordToValueRef(def.args) : undefined;
    const calculated_overrides = def.calculated_overrides;
    const condition = def.condition !== undefined ? conditionToValueRef(def.condition) : undefined;
    const until = def.until !== undefined ? conditionToValueRef(def.until) : undefined;
    this._nodes.push({
      id,
      type: def.type,
      chain: def.chain,
      skill: def.skill,
      action: def.action,
      query: def.query,
      args,
      calculated_overrides,
      condition,
      until,
      retry: def.retry,
      timeout_ms: def.timeout_ms,
      deps: def.requires,
    });
    return this;
  }

  /** Add an action node */
  action(
    id: string,
    skill: string,
    action: string,
    options?: Omit<NodeDef, 'type' | 'skill' | 'action'>
  ): this {
    return this.node(id, { type: 'action_ref', skill, action, ...options });
  }

  /** Add a query node */
  query(
    id: string,
    skill: string,
    queryName: string,
    options?: Omit<NodeDef, 'type' | 'skill' | 'query'>
  ): this {
    return this.node(id, { type: 'query_ref', skill, query: queryName, ...options });
  }

  /** Add an output mapping */
  output(name: string, ref: string | ValueRef): this {
    this._outputs[name] = typeof ref === 'string' ? { ref } : ref;
    return this;
  }

  /** Set required pack */
  requiresPack(name: string, version: string): this {
    this._requiresPack = { name, version };
    return this;
  }

  /** Set workflow policy */
  policy(policy: Workflow['policy']): this {
    this._policy = policy;
    return this;
  }

  /** Enable preflight simulation */
  preflight(config: Workflow['preflight']): this {
    this._preflight = config;
    return this;
  }

  protected getData(): Workflow {
    return {
      schema: 'ais-flow/0.0.2',
      meta: this._meta,
      default_chain: this._defaultChain,
      requires_pack: this._requiresPack,
      inputs: Object.keys(this._inputs).length > 0 ? this._inputs : undefined,
      nodes: this._nodes,
      policy: this._policy,
      preflight: this._preflight,
      outputs: Object.keys(this._outputs).length > 0 ? this._outputs : undefined,
    };
  }
}

function isValueRef(v: unknown): v is ValueRef {
  if (!v || typeof v !== 'object') return false;
  const keys = Object.keys(v as Record<string, unknown>);
  if (keys.length !== 1) return false;
  const k = keys[0];
  return (
    k === 'lit' ||
    k === 'ref' ||
    k === 'cel' ||
    k === 'detect' ||
    k === 'object' ||
    k === 'array'
  );
}

function stringToValueRef(s: string): ValueRef {
  const m = s.match(/^\$\{(.+)\}$/);
  if (m) return { ref: m[1]!.trim() };
  return { lit: s };
}

function toValueRef(v: unknown): ValueRef {
  if (isValueRef(v)) return v;
  if (typeof v === 'string') return stringToValueRef(v);
  if (Array.isArray(v)) {
    if (v.every(isValueRef)) return { array: v };
    return { lit: v };
  }
  if (v && typeof v === 'object') {
    const record = v as Record<string, unknown>;
    const values = Object.values(record);
    if (values.length > 0 && values.every(isValueRef)) {
      const out: Record<string, ValueRef> = {};
      for (const [k, vv] of Object.entries(record)) out[k] = vv as ValueRef;
      return { object: out };
    }
  }
  return { lit: v };
}

function mapRecordToValueRef(record: Record<string, unknown>): Record<string, ValueRef> {
  const out: Record<string, ValueRef> = {};
  for (const [k, v] of Object.entries(record)) out[k] = toValueRef(v);
  return out;
}

function conditionToValueRef(v: unknown): ValueRef {
  if (isValueRef(v)) return v;
  if (typeof v === 'string') {
    const m = v.match(/^\$\{(.+)\}$/);
    if (m) return { ref: m[1]!.trim() };
    return { cel: v };
  }
  return toValueRef(v);
}

/**
 * Create a new Workflow builder
 */
export function workflow(name: string, version: string): WorkflowBuilder {
  return new WorkflowBuilder(name, version);
}
