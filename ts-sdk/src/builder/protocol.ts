/**
 * Protocol Spec Builder - DSL for building protocol specs programmatically
 */

import { BaseBuilder, type ParamDef, type OutputDef } from './base.js';
import {
  ProtocolSpecSchema,
  type ProtocolSpec,
  type Action,
  type Query,
  type ExecutionBlock,
  type HardConstraints,
  type CalculatedFields,
  type RiskTag,
} from '../schema/index.js';
import type { ZodSchema } from 'zod';

// ═══════════════════════════════════════════════════════════════════════════════
// Action Builder
// ═══════════════════════════════════════════════════════════════════════════════

interface ActionDef {
  description: string;
  risk_level: number;
  execution: ExecutionBlock;
  risk_tags?: RiskTag[];
  params?: ParamDef[];
  returns?: OutputDef[];
  requires_queries?: string[];
  hard_constraints?: HardConstraints;
  calculated_fields?: CalculatedFields;
  pre_conditions?: string[];
  side_effects?: string[];
  capabilities_required?: string[];
}

// ═══════════════════════════════════════════════════════════════════════════════
// Query Builder
// ═══════════════════════════════════════════════════════════════════════════════

interface QueryDef {
  description: string;
  execution: ExecutionBlock;
  params?: ParamDef[];
  returns?: OutputDef[];
  cache_ttl?: number;
  consistency?: {
    block_tag?: 'latest' | 'safe' | 'finalized' | number;
    require_same_block?: boolean;
  };
}

// ═══════════════════════════════════════════════════════════════════════════════
// Protocol Builder
// ═══════════════════════════════════════════════════════════════════════════════

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
    const action: Action = {
      description: def.description,
      risk_level: def.risk_level,
      execution: def.execution,
      risk_tags: def.risk_tags,
      params: def.params?.map((p) => ({
        name: p.name,
        type: p.type,
        description: p.description ?? '',
        required: p.required ?? true,
        default: p.default,
        example: p.example,
        constraints: p.constraints,
        asset_ref: p.asset_ref,
      })),
      returns: def.returns?.map((o) => ({
        name: o.name,
        type: o.type,
        description: o.description,
      })),
      requires_queries: def.requires_queries,
      hard_constraints: def.hard_constraints,
      calculated_fields: def.calculated_fields,
      pre_conditions: def.pre_conditions,
      side_effects: def.side_effects,
      capabilities_required: def.capabilities_required,
    };
    this._actions[id] = action;
    return this;
  }

  /** Add a query */
  query(id: string, def: QueryDef): this {
    const query: Query = {
      description: def.description,
      execution: def.execution,
      params: def.params?.map((p) => ({
        name: p.name,
        type: p.type,
        description: p.description ?? '',
        required: p.required ?? true,
        default: p.default,
        example: p.example,
        constraints: p.constraints,
        asset_ref: p.asset_ref,
      })),
      returns: def.returns?.map((o) => ({
        name: o.name,
        type: o.type,
        description: o.description,
      })),
      cache_ttl: def.cache_ttl,
      consistency: def.consistency,
    };
    this._queries[id] = query;
    return this;
  }

  /** Add required capabilities */
  capabilities(...caps: string[]): this {
    this._capabilities.push(...caps);
    return this;
  }

  protected getData(): ProtocolSpec {
    return {
      schema: 'ais/0.0.2',
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
