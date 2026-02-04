/**
 * Resolver context - state container for resolution operations
 */
import type { ProtocolSpec } from '../schema/index.js';

export interface ResolverContext {
  /** Loaded protocol specs by protocol name */
  protocols: Map<string, ProtocolSpec>;
  /** Runtime variables */
  variables: Record<string, unknown>;
  /** Query results cache */
  queryResults: Map<string, Record<string, unknown>>;
}

/**
 * Create a fresh resolver context
 */
export function createContext(): ResolverContext {
  return {
    protocols: new Map(),
    variables: {},
    queryResults: new Map(),
  };
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
