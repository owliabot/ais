/**
 * Reference resolution - resolve protocol, action, and query references
 */
import type { ProtocolSpec, Action, Query, Pack } from '../schema/index.js';
import type { ResolverContext } from './context.js';

/**
 * Register a protocol spec in the context
 */
export function registerProtocol(ctx: ResolverContext, spec: ProtocolSpec): void {
  ctx.protocols.set(spec.protocol.name, spec);
}

/**
 * Resolve a protocol reference (e.g., "uniswap-v3" or "uniswap-v3@1.0.0")
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

  const action = spec.actions.find((a) => a.name === actionName);
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

  const query = spec.queries.find((q) => q.name === queryName);
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
