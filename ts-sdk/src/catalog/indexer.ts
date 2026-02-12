import type { Pack } from '../schema/index.js';
import type { Catalog, ActionCard, QueryCard, PackCard } from './schema.js';

export const CatalogIndexSchemaVersion = 'ais-catalog-index/0.0.1' as const;

export type DetectProviderCandidate = {
  kind: string;
  provider: string;
  chains?: string[];
  priority?: number;
};

export type ExecutionPluginCandidate = {
  type: string;
  chains?: string[];
};

export type CatalogIndex = {
  schema: typeof CatalogIndexSchemaVersion;
  catalog_schema: Catalog['schema'];
  catalog_hash: string;
  actions: ActionCard[];
  queries: QueryCard[];
  packs: PackCard[];

  // Fast lookup
  actions_by_ref: Map<string, ActionCard>;
  queries_by_ref: Map<string, QueryCard>;
  packs_by_key: Map<string, PackCard>;
  actions_by_protocol_version: Map<string, ActionCard[]>;
  queries_by_protocol_version: Map<string, QueryCard[]>;

  // Optional: derived executable candidates from a specific pack
  detect_providers?: DetectProviderCandidate[];
  execution_plugins?: ExecutionPluginCandidate[];
};

export type EngineCapabilities = {
  capabilities?: string[]; // generic capability strings
  execution_types?: string[]; // e.g. evm_call, solana_instruction, ...
  detect_kinds?: string[]; // e.g. token, address, ...
};

export function buildCatalogIndex(catalog: Catalog): CatalogIndex {
  return buildIndexFromParts({
    catalog_schema: catalog.schema,
    catalog_hash: catalog.hash,
    actions: catalog.actions ?? [],
    queries: catalog.queries ?? [],
    packs: catalog.packs ?? [],
  });
}

function buildIndexFromParts(args: {
  catalog_schema: Catalog['schema'];
  catalog_hash: string;
  actions: ActionCard[];
  queries: QueryCard[];
  packs: PackCard[];
}): CatalogIndex {
  const actions = args.actions.slice();
  const queries = args.queries.slice();
  const packs = args.packs.slice();

  const actions_by_ref = new Map<string, ActionCard>();
  const queries_by_ref = new Map<string, QueryCard>();
  const packs_by_key = new Map<string, PackCard>();
  const actions_by_protocol_version = new Map<string, ActionCard[]>();
  const queries_by_protocol_version = new Map<string, QueryCard[]>();

  for (const a of actions) {
    actions_by_ref.set(a.ref, a);
    const key = `${a.protocol}@${a.version}`;
    actions_by_protocol_version.set(key, [...(actions_by_protocol_version.get(key) ?? []), a]);
  }
  for (const q of queries) {
    queries_by_ref.set(q.ref, q);
    const key = `${q.protocol}@${q.version}`;
    queries_by_protocol_version.set(key, [...(queries_by_protocol_version.get(key) ?? []), q]);
  }
  for (const p of packs) {
    packs_by_key.set(`${p.name}@${p.version}`, p);
  }

  return {
    schema: CatalogIndexSchemaVersion,
    catalog_schema: args.catalog_schema,
    catalog_hash: args.catalog_hash,
    actions,
    queries,
    packs,
    actions_by_ref,
    queries_by_ref,
    packs_by_key,
    actions_by_protocol_version,
    queries_by_protocol_version,
  };
}

export function filterByPack(index: CatalogIndex, pack: Pack): CatalogIndex {
  const includeByProtoVer = new Map<string, { chain_scope?: string[] }>();
  for (const inc of pack.includes ?? []) {
    includeByProtoVer.set(`${inc.protocol}@${inc.version}`, {
      chain_scope: inc.chain_scope?.slice(),
    });
  }

  const actions: ActionCard[] = [];
  for (const a of index.actions) {
    const include = includeByProtoVer.get(`${a.protocol}@${a.version}`);
    if (!include) continue;
    const chainScope = include.chain_scope;
    if (Array.isArray(chainScope) && chainScope.length > 0) {
      const allowedChains = a.execution_chains.filter((c) => chainScope.includes(c));
      if (allowedChains.length === 0) continue;
      actions.push({ ...a, execution_chains: allowedChains });
    } else {
      actions.push(a);
    }
  }

  const queries: QueryCard[] = [];
  for (const q of index.queries) {
    const include = includeByProtoVer.get(`${q.protocol}@${q.version}`);
    if (!include) continue;
    const chainScope = include.chain_scope;
    if (Array.isArray(chainScope) && chainScope.length > 0) {
      const allowedChains = q.execution_chains.filter((c) => chainScope.includes(c));
      if (allowedChains.length === 0) continue;
      queries.push({ ...q, execution_chains: allowedChains });
    } else {
      queries.push(q);
    }
  }

  // Derived candidates (pack boundary)
  const detect_providers: DetectProviderCandidate[] = (pack.providers?.detect?.enabled ?? [])
    .map((e) => ({
      kind: String(e.kind),
      provider: String(e.provider),
      chains: e.chains?.slice(),
      priority: e.priority ?? 0,
    }))
    .sort((a, b) => a.kind.localeCompare(b.kind) || (b.priority ?? 0) - (a.priority ?? 0) || a.provider.localeCompare(b.provider));

  const execution_plugins: ExecutionPluginCandidate[] = (pack.plugins?.execution?.enabled ?? [])
    .map((e) => ({ type: e.type, chains: e.chains?.slice() }))
    .sort((a, b) => a.type.localeCompare(b.type));

  const out = buildIndexFromParts({
    catalog_schema: index.catalog_schema,
    catalog_hash: index.catalog_hash,
    actions,
    queries,
    packs: index.packs,
  });
  out.detect_providers = detect_providers;
  out.execution_plugins = execution_plugins;
  return out;
}

export function filterByEngineCapabilities(index: CatalogIndex, capabilities: EngineCapabilities): CatalogIndex {
  const supportedCaps = new Set((capabilities.capabilities ?? []).filter(Boolean));
  const supportedExecTypes = new Set((capabilities.execution_types ?? []).filter(Boolean));
  const supportedDetectKinds = new Set((capabilities.detect_kinds ?? []).filter(Boolean));

  const actions = index.actions.filter((a) => {
    const req = a.capabilities_required ?? [];
    if (req.length > 0 && supportedCaps.size > 0) {
      for (const c of req) if (!supportedCaps.has(c)) return false;
    }
    if (supportedExecTypes.size > 0) {
      // Conservative: require all declared execution_types to be supported.
      for (const t of a.execution_types) if (!supportedExecTypes.has(t)) return false;
    }
    return true;
  });

  const queries = index.queries.filter((q) => {
    const req = q.capabilities_required ?? [];
    if (req.length > 0 && supportedCaps.size > 0) {
      for (const c of req) if (!supportedCaps.has(c)) return false;
    }
    if (supportedExecTypes.size > 0) {
      for (const t of q.execution_types) if (!supportedExecTypes.has(t)) return false;
    }
    return true;
  });

  const out = buildIndexFromParts({
    catalog_schema: index.catalog_schema,
    catalog_hash: index.catalog_hash,
    actions,
    queries,
    packs: index.packs,
  });

  if (index.detect_providers) {
    out.detect_providers =
      supportedDetectKinds.size > 0 ? index.detect_providers.filter((p) => supportedDetectKinds.has(p.kind)) : index.detect_providers.slice();
  }
  if (index.execution_plugins) {
    out.execution_plugins =
      supportedExecTypes.size > 0 ? index.execution_plugins.filter((p) => supportedExecTypes.has(p.type)) : index.execution_plugins.slice();
  }

  return out;
}
