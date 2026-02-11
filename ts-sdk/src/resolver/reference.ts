/**
 * Reference resolution - resolve protocol, action, and query references
 */
import type { ProtocolSpec, Action, Query, Pack } from '../schema/index.js';
import type { ProtocolSource, ResolverContext } from './context.js';

/**
 * Register a protocol spec in the context
 */
export function registerProtocol(
  ctx: ResolverContext,
  spec: ProtocolSpec,
  options?: { source?: ProtocolSource }
): void {
  ctx.protocols.set(spec.meta.protocol, spec);
  ctx.protocol_sources.set(spec.meta.protocol, options?.source ?? 'manual');
}

/**
 * Parse a protocol reference (e.g., "uniswap-v3@1.0.0" → { protocol: "uniswap-v3", version: "1.0.0" })
 */
export function parseProtocolRef(ref: string): { protocol: string; version?: string } {
  const [protocol, version] = ref.split('@');
  return { protocol, version };
}

/**
 * Resolve a protocol reference (e.g., "uniswap-v3" or "uniswap-v3@1.0.0")
 */
export function resolveProtocolRef(
  ctx: ResolverContext,
  ref: string
): ProtocolSpec | null {
  const { protocol, version } = parseProtocolRef(ref);
  const spec = ctx.protocols.get(protocol);
  if (!spec) return null;
  if (version && spec.meta.version !== version) return null;
  return spec;
}

/**
 * Find an action by reference (e.g., "uniswap-v3/swap_exact_in" or "uniswap-v3@1.0.0/swap_exact_in")
 */
export function resolveAction(
  ctx: ResolverContext,
  ref: string
): { protocol: ProtocolSpec; actionId: string; action: Action } | null {
  const [protocolPart, actionId] = ref.split('/');
  if (!actionId) return null;

  const spec = resolveProtocolRef(ctx, protocolPart);
  if (!spec) return null;

  const action = spec.actions[actionId];
  if (!action) return null;

  return { protocol: spec, actionId, action };
}

/**
 * Find a query by reference (e.g., "uniswap-v3/get_pool")
 */
export function resolveQuery(
  ctx: ResolverContext,
  ref: string
): { protocol: ProtocolSpec; queryId: string; query: Query } | null {
  const [protocolPart, queryId] = ref.split('/');
  if (!queryId) return null;

  const spec = resolveProtocolRef(ctx, protocolPart);
  if (!spec || !spec.queries) return null;

  const query = spec.queries[queryId];
  if (!query) return null;

  return { protocol: spec, queryId, query };
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

  for (const protocolInclude of pack.includes) {
    const ref = `${protocolInclude.protocol}@${protocolInclude.version}`;
    const spec = resolveProtocolRef(ctx, ref);
    if (spec) {
      protocols.push(spec);
    } else {
      missing.push(ref);
    }
  }

  return { protocols, missing };
}

/**
 * Get contract address for a protocol on a specific chain
 */
export function getContractAddress(
  spec: ProtocolSpec,
  chain: string,
  contractName: string
): string | null {
  const deployment = spec.deployments.find((d) => d.chain === chain);
  if (!deployment) return null;
  return deployment.contracts[contractName] ?? null;
}

/**
 * Get all supported chains for a protocol
 */
export function getSupportedChains(spec: ProtocolSpec): string[] {
  return spec.deployments.map((d) => d.chain);
}
