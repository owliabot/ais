/**
 * Reference resolution - resolve protocol, action, and query references
 */
import type { ProtocolSpec, Action, Query, Pack } from '../schema/index.js';
import type { ResolverContext } from './context.js';

/**
 * Register a protocol spec in the context
 */
export function registerProtocol(ctx: ResolverContext, spec: ProtocolSpec): void {
  ctx.protocols.set(spec.meta.protocol, spec);
}

/**
 * Parse a skill reference (e.g., "uniswap-v3@1.0.0" → { protocol: "uniswap-v3", version: "1.0.0" })
 */
export function parseSkillRef(ref: string): { protocol: string; version?: string } {
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
  const { protocol, version } = parseSkillRef(ref);
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
  const [skillPart, actionId] = ref.split('/');
  if (!actionId) return null;

  const spec = resolveProtocolRef(ctx, skillPart);
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
  const [skillPart, queryId] = ref.split('/');
  if (!queryId) return null;

  const spec = resolveProtocolRef(ctx, skillPart);
  if (!spec || !spec.queries) return null;

  const query = spec.queries[queryId];
  if (!query) return null;

  return { protocol: spec, queryId, query };
}

/**
 * Expand all skill references in a Pack
 */
export function expandPack(
  ctx: ResolverContext,
  pack: Pack
): { protocols: ProtocolSpec[]; missing: string[] } {
  const protocols: ProtocolSpec[] = [];
  const missing: string[] = [];

  for (const ref of pack.includes) {
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
