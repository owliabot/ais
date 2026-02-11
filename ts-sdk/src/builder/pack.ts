/**
 * Pack Builder - DSL for building pack configs programmatically
 */

import { BaseBuilder } from './base.js';
import {
  PackSchema,
  type Pack,
  type ProtocolInclude,
  type Policy,
  type TokenPolicy,
  type TokenAllowlistEntry,
  type HardConstraintsDefaults,
} from '../schema/index.js';
import type { ZodSchema } from 'zod';

export class PackBuilder extends BaseBuilder<Pack> {
  protected schema: ZodSchema<Pack> = PackSchema;

  private _name: string;
  private _version: string;
  private _description?: string;
  private _includes: ProtocolInclude[] = [];
  private _policy?: Policy;
  private _tokenPolicy?: TokenPolicy;
  private _providers?: Pack['providers'];

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

  /** Include a protocol */
  include(
    protocol: string,
    version: string,
    options?: {
      source?: 'registry' | 'local' | 'uri';
      uri?: string;
      chain_scope?: string[];
    }
  ): this {
    this._includes.push({
      protocol,
      version,
      ...options,
    });
    return this;
  }

  /** Set approval policy */
  approvals(config: {
    auto_execute_max_risk_level?: number;
    require_approval_min_risk_level?: number;
  }): this {
    this._policy = {
      ...this._policy,
      approvals: config,
    };
    return this;
  }

  /** Set hard constraints defaults */
  constraints(constraints: HardConstraintsDefaults): this {
    this._policy = {
      ...this._policy,
      hard_constraints_defaults: constraints,
    };
    return this;
  }

  /** Set maximum slippage in basis points */
  maxSlippage(bps: number): this {
    this._policy = {
      ...this._policy,
      hard_constraints_defaults: {
        ...this._policy?.hard_constraints_defaults,
        max_slippage_bps: bps,
      },
    };
    return this;
  }

  /** Disallow unlimited approvals */
  disallowUnlimitedApproval(): this {
    this._policy = {
      ...this._policy,
      hard_constraints_defaults: {
        ...this._policy?.hard_constraints_defaults,
        allow_unlimited_approval: false,
      },
    };
    return this;
  }

  /** Set token resolution policy */
  tokenResolution(config: {
    allow_symbol_input?: boolean;
    require_user_confirm_asset_address?: boolean;
    require_allowlist_for_symbol_resolution?: boolean;
  }): this {
    this._tokenPolicy = {
      ...this._tokenPolicy,
      resolution: config,
    };
    return this;
  }

  /** Add to token allowlist */
  allowToken(entry: TokenAllowlistEntry): this {
    this._tokenPolicy = {
      ...this._tokenPolicy,
      allowlist: [...(this._tokenPolicy?.allowlist ?? []), entry],
    };
    return this;
  }

  /** Set full token policy */
  tokens(policy: TokenPolicy): this {
    this._tokenPolicy = policy;
    return this;
  }

  /** Add quote provider */
  quoteProvider(
    provider: string,
    options?: { chains?: string[]; priority?: number }
  ): this {
    const existing = this._providers?.quote?.enabled ?? [];
    this._providers = {
      ...this._providers,
      quote: {
        enabled: [...existing, { provider, ...options }],
      },
    };
    return this;
  }

  /** Add routing providers */
  routingProviders(...providers: string[]): this {
    this._providers = {
      ...this._providers,
      routing: [...(this._providers?.routing ?? []), ...providers],
    };
    return this;
  }

  protected getData(): Pack {
    return {
      schema: 'ais-pack/0.0.2',
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
