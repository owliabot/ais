/**
 * Workflow Builder
 */

import { BaseBuilder } from './base.js';
import { WorkflowSchema, type Workflow, type WorkflowNode, type WorkflowInput } from '../schema/index.js';
import type { ZodSchema } from 'zod';

interface NodeDef {
  type: 'action_ref' | 'query_ref';
  skill: string;
  action?: string;
  query?: string;
  args?: Record<string, unknown>;
  condition?: string;
  requires?: string[];
}

export class WorkflowBuilder extends BaseBuilder<Workflow> {
  protected schema: ZodSchema<Workflow> = WorkflowSchema;

  private _meta: Workflow['meta'];
  private _inputs: Record<string, WorkflowInput> = {};
  private _nodes: WorkflowNode[] = [];
  private _outputs: Record<string, string> = {};
  private _requiresPack?: { name: string; version: string };

  constructor(name: string, version: string) {
    super();
    this._meta = { name, version };
  }

  /** Set workflow description */
  description(desc: string): this {
    this._meta.description = desc;
    return this;
  }

  /** Add an input parameter */
  input(name: string, type: string, options?: Omit<WorkflowInput, 'type'>): this {
    this._inputs[name] = { type, ...options };
    return this;
  }

  /** Add a required input parameter */
  requiredInput(name: string, type: string, options?: Omit<WorkflowInput, 'type' | 'required'>): this {
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
    this._nodes.push({
      id,
      type: def.type,
      skill: def.skill,
      action: def.action,
      query: def.query,
      args: def.args,
      condition: def.condition,
      requires_queries: def.requires,
    });
    return this;
  }

  /** Add an action node */
  action(id: string, skill: string, action: string, options?: Omit<NodeDef, 'type' | 'skill' | 'action'>): this {
    return this.node(id, { type: 'action_ref', skill, action, ...options });
  }

  /** Add a query node */
  query(id: string, skill: string, queryName: string, options?: Omit<NodeDef, 'type' | 'skill' | 'query'>): this {
    return this.node(id, { type: 'query_ref', skill, query: queryName, ...options });
  }

  /** Add an output mapping */
  output(name: string, ref: string): this {
    this._outputs[name] = ref;
    return this;
  }

  /** Set required pack */
  requiresPack(name: string, version: string): this {
    this._requiresPack = { name, version };
    return this;
  }

  protected getData(): Workflow {
    return {
      schema: 'ais-flow/1.0',
      meta: this._meta,
      requires_pack: this._requiresPack,
      inputs: Object.keys(this._inputs).length > 0 ? this._inputs : undefined,
      nodes: this._nodes,
      outputs: Object.keys(this._outputs).length > 0 ? this._outputs : undefined,
    };
  }
}

/**
 * Create a new Workflow builder
 */
export function workflow(name: string, version: string): WorkflowBuilder {
  return new WorkflowBuilder(name, version);
}
