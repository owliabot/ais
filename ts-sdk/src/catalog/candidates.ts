import { createHash } from 'node:crypto';
import type { Pack } from '../schema/index.js';
import type { Catalog, ActionCard, QueryCard } from './schema.js';
import {
  buildCatalogIndex,
  filterByEngineCapabilities,
  filterByPack,
  type CatalogIndex,
  type DetectProviderCandidate,
  type EngineCapabilities,
  type ExecutionPluginCandidate,
} from './indexer.js';

export const ExecutableCandidatesSchemaVersion = 'ais-executable-candidates/0.0.1' as const;

export type ExecutableActionCandidate = ActionCard & {
  signature: string;
};

export type ExecutableQueryCandidate = QueryCard & {
  signature: string;
};

export type ExecutableDetectProviderCandidate = {
  kind: string;
  provider: string;
  chain?: string;
  priority: number;
};

export type ExecutableExecutionPluginCandidate = {
  type: string;
  chain?: string;
};

export type ExecutableCandidates = {
  schema: typeof ExecutableCandidatesSchemaVersion;
  created_at: string;
  hash: string;
  catalog_schema: Catalog['schema'];
  catalog_hash: string;
  pack?: { name: string; version: string };
  chain_scope?: string[];
  actions: ExecutableActionCandidate[];
  queries: ExecutableQueryCandidate[];
  detect_providers: ExecutableDetectProviderCandidate[];
  execution_plugins: ExecutableExecutionPluginCandidate[];
  extensions?: Record<string, unknown>;
};

export function getExecutableCandidates(args: {
  catalog: Catalog;
  pack?: Pack;
  engine_capabilities?: EngineCapabilities;
  chain_scope?: string[];
}): ExecutableCandidates {
  const { catalog, pack, engine_capabilities, chain_scope } = args;

  let idx: CatalogIndex = buildCatalogIndex(catalog);
  if (pack) idx = filterByPack(idx, pack);
  if (engine_capabilities) idx = filterByEngineCapabilities(idx, engine_capabilities);

  const scoped = Array.isArray(chain_scope) && chain_scope.length > 0 ? chain_scope.slice() : undefined;
  if (scoped) {
    idx = applyChainScope(idx, scoped);
  }

  const actions = idx.actions
    .map((a) => ({ ...a, signature: cardSignature(a.id, a.params, a.returns) }))
    .sort((a, b) => a.ref.localeCompare(b.ref));

  const queries = idx.queries
    .map((q) => ({ ...q, signature: cardSignature(q.id, q.params, q.returns) }))
    .sort((a, b) => a.ref.localeCompare(b.ref));

  const detect_providers = explodeDetectProviders(idx.detect_providers ?? [])
    .filter((p) => !scoped || !p.chain || scoped.includes(p.chain))
    .sort(
      (a, b) =>
        a.kind.localeCompare(b.kind) ||
        String(a.chain ?? '').localeCompare(String(b.chain ?? '')) ||
        b.priority - a.priority ||
        a.provider.localeCompare(b.provider)
    );

  const execution_plugins = explodeExecutionPlugins(idx.execution_plugins ?? [])
    .filter((p) => !scoped || !p.chain || scoped.includes(p.chain))
    .sort((a, b) => a.type.localeCompare(b.type) || String(a.chain ?? '').localeCompare(String(b.chain ?? '')));

  const created_at = new Date().toISOString();

  const packId = pack
    ? {
        name: pack.meta?.name ?? pack.name ?? '(unknown-pack)',
        version: pack.meta?.version ?? pack.version ?? '(unknown-version)',
      }
    : undefined;

  const contentForHash = {
    schema: ExecutableCandidatesSchemaVersion,
    catalog_schema: catalog.schema,
    catalog_hash: catalog.hash,
    pack: packId,
    chain_scope: scoped,
    actions,
    queries,
    detect_providers,
    execution_plugins,
  };
  const hash = sha256Hex(stableJsonStringify(contentForHash));

  return {
    schema: ExecutableCandidatesSchemaVersion,
    created_at,
    hash,
    catalog_schema: catalog.schema,
    catalog_hash: catalog.hash,
    ...(packId ? { pack: packId } : {}),
    ...(scoped ? { chain_scope: scoped.slice().sort() } : {}),
    actions,
    queries,
    detect_providers,
    execution_plugins,
  };
}

function applyChainScope(index: CatalogIndex, chain_scope: string[]): CatalogIndex {
  const scoped = chain_scope.slice();
  const actions = index.actions
    .map((a) => ({
      ...a,
      execution_chains: a.execution_chains.filter((c) => scoped.includes(c)),
    }))
    .filter((a) => a.execution_chains.length > 0);

  const queries = index.queries
    .map((q) => ({
      ...q,
      execution_chains: q.execution_chains.filter((c) => scoped.includes(c)),
    }))
    .filter((q) => q.execution_chains.length > 0);

  // Keep pack list unchanged.
  const out = buildCatalogIndex({
    schema: index.catalog_schema,
    created_at: '(scoped)',
    hash: index.catalog_hash,
    actions,
    queries,
    packs: index.packs,
  } as any);

  out.detect_providers = index.detect_providers ? index.detect_providers.slice() : undefined;
  out.execution_plugins = index.execution_plugins ? index.execution_plugins.slice() : undefined;
  return out;
}

function explodeDetectProviders(providers: DetectProviderCandidate[]): ExecutableDetectProviderCandidate[] {
  const out: ExecutableDetectProviderCandidate[] = [];
  for (const p of providers) {
    const priority = typeof p.priority === 'number' ? p.priority : 0;
    if (Array.isArray(p.chains) && p.chains.length > 0) {
      for (const chain of p.chains) {
        out.push({ kind: p.kind, provider: p.provider, chain, priority });
      }
    } else {
      out.push({ kind: p.kind, provider: p.provider, priority });
    }
  }
  return out;
}

function explodeExecutionPlugins(plugins: ExecutionPluginCandidate[]): ExecutableExecutionPluginCandidate[] {
  const out: ExecutableExecutionPluginCandidate[] = [];
  for (const p of plugins) {
    if (Array.isArray(p.chains) && p.chains.length > 0) {
      for (const chain of p.chains) out.push({ type: p.type, chain });
    } else {
      out.push({ type: p.type });
    }
  }
  return out;
}

function cardSignature(id: string, params?: Array<{ name: string; type: string }>, returns?: Array<{ name: string; type: string }>): string {
  const ps = Array.isArray(params) ? params.map((p) => `${p.name}:${p.type}`).join(',') : '';
  const rs = Array.isArray(returns) ? returns.map((r) => `${r.name}:${r.type}`).join(',') : '';
  return `${id}(${ps})${rs ? `->(${rs})` : ''}`;
}

function stableJsonStringify(value: unknown): string {
  return JSON.stringify(sortKeysDeep(value));
}

function sortKeysDeep(value: any): any {
  if (Array.isArray(value)) return value.map((v) => sortKeysDeep(v));
  if (!value || typeof value !== 'object') return value;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) {
    out[key] = sortKeysDeep(value[key]);
  }
  return out;
}

function sha256Hex(text: string): string {
  return createHash('sha256').update(text).digest('hex');
}

