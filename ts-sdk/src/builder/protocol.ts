/**
 * Protocol Spec Builder
 */

import { BaseBuilder, type ParamDef, type OutputDef } from './base.js';
import { ProtocolSpecSchema, type ProtocolSpec, type Action, type Query } from '../schema/index.js';
import type { ZodSchema } from 'zod';

interface ActionDef {
  contract: string;
  method: string;
  description?: string;
  params?: ParamDef[];
  outputs?: OutputDef[];
  requires_queries?: string[];
}

interface QueryDef {
  contract: string;
  method: string;
  description?: string;
  params?: ParamDef[];
  outputs?: OutputDef[];
}

export class ProtocolBuilder extends BaseBuilder<ProtocolSpec> {
  protected schema: ZodSchema<ProtocolSpec> = ProtocolSpecSchema;

  private _meta: ProtocolSpec['meta'];
  private _deployments: ProtocolSpec['deployments'] = [];
  private _actions: Record<string, Action> = {};
  private _queries: Record<string, Query> = {};
  private _capabilities: string[] = [];

  constructor(name: string, version: string) {
    super();
    this._meta = { protocol: name, version };
  }

  /** Set protocol description */
  description(desc: string): this {
    this._meta.description = desc;
    return this;
  }

  /** Set protocol name (display name) */
  name(displayName: string): this {
    this._meta.name = displayName;
    return this;
  }

  /** Set homepage URL */
  homepage(url: string): this {
    this._meta.homepage = url;
    return this;
  }

  /** Set maintainer */
  maintainer(maintainer: string): this {
    this._meta.maintainer = maintainer;
    return this;
  }

  /** Add tags */
  tags(...tags: string[]): this {
    this._meta.tags = [...(this._meta.tags ?? []), ...tags];
    return this;
  }

  /** Add a deployment for a chain */
  deployment(chain: string, contracts: Record<string, string>): this {
    this._deployments.push({ chain, contracts });
    return this;
  }

  /** Add an action */
  action(id: string, def: ActionDef): this {
    this._actions[id] = {
      contract: def.contract,
      method: def.method,
      description: def.description,
      params: def.params,
      outputs: def.outputs,
      requires_queries: def.requires_queries,
    };
    return this;
  }

  /** Add a query */
  query(id: string, def: QueryDef): this {
    this._queries[id] = {
      contract: def.contract,
      method: def.method,
      description: def.description,
      params: def.params,
      outputs: def.outputs,
    };
    return this;
  }

  /** Add required capabilities */
  capabilities(...caps: string[]): this {
    this._capabilities.push(...caps);
    return this;
  }

  protected getData(): ProtocolSpec {
    return {
      schema: 'ais/1.0',
      meta: this._meta,
      deployments: this._deployments,
      actions: this._actions,
      queries: Object.keys(this._queries).length > 0 ? this._queries : undefined,
      capabilities_required: this._capabilities.length > 0 ? this._capabilities : undefined,
    };
  }
}

/**
 * Create a new Protocol builder
 */
export function protocol(name: string, version: string): ProtocolBuilder {
  return new ProtocolBuilder(name, version);
}
