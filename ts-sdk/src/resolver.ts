/**
 * AIS Protocol SDK - Resolver
 * Resolve protocol references and expression placeholders
 */

import type { ProtocolSpec, Pack, Action, Query } from './types.js';

export interface ResolverContext {
  /** Loaded protocol specs by name */
  protocols: Map<string, ProtocolSpec>;
  /** Runtime variables */
  variables: Record<string, unknown>;
  /** Query results cache */
  queryResults: Map<string, Record<string, unknown>>;
}

export function createContext(): ResolverContext {
  return {
    protocols: new Map(),
    variables: {},
    queryResults: new Map(),
  };
}

/**
 * Register a protocol spec in the context
 */
export function registerProtocol(ctx: ResolverContext, spec: ProtocolSpec): void {
  ctx.protocols.set(spec.protocol.name, spec);
}

/**
 * Resolve a protocol reference (e.g., "uniswap-v3@1.0.0")
 */
export function resolveProtocolRef(
  ctx: ResolverContext,
  ref: string
): ProtocolSpec | null {
  const [name, version] = ref.split('@');
  const spec = ctx.protocols.get(name);
  if (!spec) return null;
  if (version && spec.protocol.version !== version) return null;
  return spec;
}

/**
 * Find an action by reference (e.g., "uniswap-v3/swap_exact_in")
 */
export function resolveAction(
  ctx: ResolverContext,
  ref: string
): { protocol: ProtocolSpec; action: Action } | null {
  const [protocolName, actionName] = ref.split('/');
  const spec = ctx.protocols.get(protocolName);
  if (!spec) return null;

  const action = spec.actions.find(a => a.name === actionName);
  if (!action) return null;

  return { protocol: spec, action };
}

/**
 * Find a query by reference (e.g., "uniswap-v3/get_pool")
 */
export function resolveQuery(
  ctx: ResolverContext,
  ref: string
): { protocol: ProtocolSpec; query: Query } | null {
  const [protocolName, queryName] = ref.split('/');
  const spec = ctx.protocols.get(protocolName);
  if (!spec || !spec.queries) return null;

  const query = spec.queries.find(q => q.name === queryName);
  if (!query) return null;

  return { protocol: spec, query };
}

/**
 * Expand all protocol references in a Pack
 */
export function expandPack(
  ctx: ResolverContext,
  pack: Pack
): { protocols: ProtocolSpec[]; missing: string[] } {
  const protocols: ProtocolSpec[] = [];
  const missing: string[] = [];

  for (const ref of pack.protocols) {
    const key = `${ref.protocol}@${ref.version}`;
    const spec = resolveProtocolRef(ctx, key);
    if (spec) {
      protocols.push(spec);
    } else {
      missing.push(key);
    }
  }

  return { protocols, missing };
}

// =============================================================================
// Expression Resolution
// =============================================================================

const EXPR_PATTERN = /\$\{([^}]+)\}/g;

/**
 * Check if a string contains expression placeholders
 */
export function hasExpressions(value: string): boolean {
  return EXPR_PATTERN.test(value);
}

/**
 * Extract all expression references from a string
 */
export function extractExpressions(value: string): string[] {
  const matches: string[] = [];
  let match: RegExpExecArray | null;
  const pattern = new RegExp(EXPR_PATTERN.source, 'g');
  while ((match = pattern.exec(value)) !== null) {
    matches.push(match[1]);
  }
  return matches;
}

/**
 * Resolve expression placeholders in a string
 * Supports: ${input.x}, ${query.name.field}, ${step.id.output}
 */
export function resolveExpression(
  expr: string,
  ctx: ResolverContext
): unknown {
  // Handle namespaced references
  const parts = expr.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'input':
    case 'inputs': {
      const key = parts.slice(1).join('.');
      return ctx.variables[key];
    }
    case 'query': {
      const queryName = parts[1];
      const field = parts.slice(2).join('.');
      const result = ctx.queryResults.get(queryName);
      if (!result) return undefined;
      return field ? result[field] : result;
    }
    case 'step': {
      const stepId = parts[1];
      const output = parts.slice(2).join('.');
      const stepKey = `step.${stepId}`;
      const result = ctx.variables[stepKey] as Record<string, unknown> | undefined;
      if (!result) return undefined;
      return output ? result[output] : result;
    }
    case 'address': {
      // ${address.router} → protocol address
      const addrName = parts[1];
      for (const spec of ctx.protocols.values()) {
        if (addrName in spec.protocol.addresses) {
          return spec.protocol.addresses[addrName];
        }
      }
      return undefined;
    }
    default:
      // Direct variable lookup
      return ctx.variables[expr];
  }
}

/**
 * Resolve all expressions in a string, returning the resolved string
 */
export function resolveExpressionString(
  template: string,
  ctx: ResolverContext
): string {
  return template.replace(EXPR_PATTERN, (_, expr) => {
    const value = resolveExpression(expr, ctx);
    return value !== undefined ? String(value) : `\${${expr}}`;
  });
}

/**
 * Set a variable in the context
 */
export function setVariable(
  ctx: ResolverContext,
  key: string,
  value: unknown
): void {
  ctx.variables[key] = value;
}

/**
 * Store query results in the context
 */
export function setQueryResult(
  ctx: ResolverContext,
  queryName: string,
  result: Record<string, unknown>
): void {
  ctx.queryResults.set(queryName, result);
}
