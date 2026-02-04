/**
 * Pack Builder
 */

import { BaseBuilder } from './base.js';
import { PackSchema, type Pack, type Policy, type TokenPolicy, type HardConstraints } from '../schema/index.js';
import type { ZodSchema } from 'zod';

export class PackBuilder extends BaseBuilder<Pack> {
  protected schema: ZodSchema<Pack> = PackSchema;

  private _name: string;
  private _version: string;
  private _description?: string;
  private _includes: string[] = [];
  private _policy?: Policy;
  private _tokenPolicy?: TokenPolicy;
  private _providers?: { quote?: string[]; routing?: string[] };

  constructor(name: string, version: string) {
    super();
    this._name = name;
    this._version = version;
  }

  /** Set pack description */
  description(desc: string): this {
    this._description = desc;
    return this;
  }

  /** Include a protocol (skill reference) */
  include(skillRef: string): this {
    this._includes.push(skillRef);
    return this;
  }

  /** Include multiple protocols */
  includes(...skillRefs: string[]): this {
    this._includes.push(...skillRefs);
    return this;
  }

  /** Set risk policy */
  policy(policy: {
    risk_threshold?: number;
    approval_required?: string[];
    hard_constraints?: HardConstraints;
  }): this {
    this._policy = policy;
    return this;
  }

  /** Set hard constraints directly */
  constraints(constraints: HardConstraints): this {
    this._policy = {
      ...this._policy,
      hard_constraints: constraints,
    };
    return this;
  }

  /** Set maximum slippage in basis points */
  maxSlippage(bps: number): this {
    this._policy = {
      ...this._policy,
      hard_constraints: {
        ...this._policy?.hard_constraints,
        max_slippage_bps: bps,
      },
    };
    return this;
  }

  /** Set token allowlist */
  tokenAllowlist(tokens: string[]): this {
    this._tokenPolicy = {
      ...this._tokenPolicy,
      allowlist: tokens,
    };
    return this;
  }

  /** Set token policy resolution mode */
  tokenResolution(mode: 'strict' | 'permissive'): this {
    this._tokenPolicy = {
      ...this._tokenPolicy,
      resolution: mode,
    };
    return this;
  }

  /** Set full token policy */
  tokens(policy: TokenPolicy): this {
    this._tokenPolicy = policy;
    return this;
  }

  /** Set quote providers */
  quoteProviders(...providers: string[]): this {
    this._providers = {
      ...this._providers,
      quote: providers,
    };
    return this;
  }

  /** Set routing providers */
  routingProviders(...providers: string[]): this {
    this._providers = {
      ...this._providers,
      routing: providers,
    };
    return this;
  }

  protected getData(): Pack {
    return {
      schema: 'ais-pack/1.0',
      name: this._name,
      version: this._version,
      description: this._description,
      includes: this._includes,
      policy: this._policy,
      token_policy: this._tokenPolicy,
      providers: this._providers,
    };
  }
}

/**
 * Create a new Pack builder
 */
export function pack(name: string, version: string): PackBuilder {
  return new PackBuilder(name, version);
}
