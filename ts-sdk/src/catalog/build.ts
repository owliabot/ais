import { createHash } from 'node:crypto';
import type { DirectoryLoadResult } from '../loader.js';
import type { Pack, ProtocolSpec } from '../schema/index.js';
import { CatalogSchemaVersion, type ActionCard, type Catalog, type CatalogDocumentEntry, type PackCard, type QueryCard } from './schema.js';

export function buildCatalog(workspace: DirectoryLoadResult): Catalog {
  const actions: ActionCard[] = [];
  const queries: QueryCard[] = [];
  const packs: PackCard[] = [];
  const documents: CatalogDocumentEntry[] = [];

  for (const entry of workspace.protocols) {
    const spec = entry.document;
    const proto = spec.meta.protocol;
    const ver = spec.meta.version;
    const docId = `${proto}@${ver}`;
    documents.push({
      path: entry.path,
      kind: 'protocol',
      id: docId,
      hash: sha256Hex(stableJsonStringify(protocolDocFingerprint(spec))),
    });

    for (const [actionId, action] of Object.entries(spec.actions ?? {})) {
      actions.push(actionCardFromProtocol(spec, actionId));
    }
    for (const [queryId, query] of Object.entries(spec.queries ?? {})) {
      void query; // only for Object.entries typing symmetry
      queries.push(queryCardFromProtocol(spec, queryId));
    }
  }

  for (const entry of workspace.packs) {
    const pack = entry.document;
    const name = pack.meta?.name ?? pack.name ?? '(unknown-pack)';
    const version = pack.meta?.version ?? pack.version ?? '(unknown-version)';
    const docId = `${name}@${version}`;
    documents.push({
      path: entry.path,
      kind: 'pack',
      id: docId,
      hash: sha256Hex(stableJsonStringify(packDocFingerprint(pack))),
    });
    packs.push(packCardFromPack(pack));
  }

  for (const entry of workspace.workflows) {
    const wf = entry.document;
    const docId = `${wf.meta.name}@${wf.meta.version}`;
    documents.push({
      path: entry.path,
      kind: 'workflow',
      id: docId,
      hash: sha256Hex(stableJsonStringify({ schema: wf.schema, name: wf.meta.name, version: wf.meta.version })),
    });
  }

  for (const err of workspace.errors) {
    documents.push({
      path: err.path,
      kind: 'error',
      hash: sha256Hex(stableJsonStringify({ kind: err.kind ?? 'unknown', error: err.error, field_path: err.field_path })),
    });
  }

  sortCards(actions, queries, packs, documents);

  const createdAt = new Date().toISOString();
  const contentForHash = {
    schema: CatalogSchemaVersion,
    actions,
    queries,
    packs,
    documents,
  };
  const hash = sha256Hex(stableJsonStringify(contentForHash));

  return {
    schema: CatalogSchemaVersion,
    created_at: createdAt,
    hash,
    documents,
    actions,
    queries,
    packs,
  };
}

function actionCardFromProtocol(spec: ProtocolSpec, actionId: string): ActionCard {
  const proto = spec.meta.protocol;
  const ver = spec.meta.version;
  const action = spec.actions[actionId]!;
  const protocolCaps = Array.isArray(spec.capabilities_required) ? spec.capabilities_required : [];
  const actionCaps = Array.isArray((action as any).capabilities_required) ? (action as any).capabilities_required : [];
  const caps = uniqStrings([...protocolCaps, ...actionCaps]).sort();
  return {
    ref: `${proto}@${ver}/${actionId}`,
    protocol: proto,
    version: ver,
    id: actionId,
    description: action.description,
    risk_level: action.risk_level,
    risk_tags: action.risk_tags?.slice()?.sort(),
    params: action.params?.map((p) => ({
      name: p.name,
      type: String(p.type),
      required: p.required,
      asset_ref: p.asset_ref,
    })),
    returns: action.returns?.map((r) => ({ name: r.name, type: String(r.type) })),
    requires_queries: action.requires_queries?.slice()?.sort(),
    capabilities_required: caps.length > 0 ? caps : undefined,
    execution_types: extractExecutionTypes(action.execution).sort(),
    execution_chains: Object.keys(action.execution ?? {}).sort(),
  };
}

function queryCardFromProtocol(spec: ProtocolSpec, queryId: string): QueryCard {
  const proto = spec.meta.protocol;
  const ver = spec.meta.version;
  const query = (spec.queries ?? {})[queryId]!;
  const protocolCaps = Array.isArray(spec.capabilities_required) ? spec.capabilities_required.slice().sort() : [];
  return {
    ref: `${proto}@${ver}/${queryId}`,
    protocol: proto,
    version: ver,
    id: queryId,
    description: query.description,
    params: query.params?.map((p) => ({
      name: p.name,
      type: String(p.type),
      required: p.required,
      asset_ref: p.asset_ref,
    })),
    returns: query.returns?.map((r) => ({ name: r.name, type: String(r.type) })),
    capabilities_required: protocolCaps.length > 0 ? protocolCaps : undefined,
    execution_types: extractExecutionTypes(query.execution).sort(),
    execution_chains: Object.keys(query.execution ?? {}).sort(),
  };
}

function packCardFromPack(pack: Pack): PackCard {
  const name = pack.meta?.name ?? pack.name ?? '(unknown-pack)';
  const version = pack.meta?.version ?? pack.version ?? '(unknown-version)';
  const includes = (pack.includes ?? []).map((inc) => ({
    protocol: inc.protocol,
    version: inc.version,
    chain_scope: inc.chain_scope?.slice()?.sort(),
  }));
  includes.sort((a, b) => a.protocol.localeCompare(b.protocol) || a.version.localeCompare(b.version));

  const policy = pack.policy
    ? {
        approvals: pack.policy.approvals ?? undefined,
        hard_constraints_defaults: pack.policy.hard_constraints_defaults ?? pack.policy.hard_constraints ?? undefined,
      }
    : undefined;

  const token_policy = pack.token_policy
    ? {
        resolution: pack.token_policy.resolution ?? undefined,
        allowlist_count: Array.isArray(pack.token_policy.allowlist) ? pack.token_policy.allowlist.length : 0,
      }
    : undefined;

  const providers = pack.providers
    ? {
        detect_enabled: pack.providers.detect?.enabled?.map((e) => ({
          kind: e.kind,
          provider: e.provider,
          chains: e.chains?.slice()?.sort(),
          priority: e.priority ?? 0,
        })).sort((a, b) => a.kind.localeCompare(b.kind) || (b.priority - a.priority) || a.provider.localeCompare(b.provider)),
        quote_enabled: pack.providers.quote?.enabled?.map((e) => ({
          provider: e.provider,
          chains: e.chains?.slice()?.sort(),
          priority: e.priority ?? 0,
        })).sort((a, b) => (b.priority - a.priority) || a.provider.localeCompare(b.provider)),
      }
    : undefined;

  const plugins = pack.plugins
    ? {
        execution_enabled: pack.plugins.execution?.enabled?.map((e) => ({
          type: e.type,
          chains: e.chains?.slice()?.sort(),
        })).sort((a, b) => a.type.localeCompare(b.type)),
      }
    : undefined;

  const overrides = pack.overrides?.actions
    ? {
        action_keys: Object.keys(pack.overrides.actions).slice().sort(),
        count: Object.keys(pack.overrides.actions).length,
      }
    : undefined;

  return {
    name,
    version,
    description: pack.meta?.description ?? pack.description,
    includes,
    ...(policy ? { policy } : {}),
    ...(token_policy ? { token_policy } : {}),
    ...(providers ? { providers } : {}),
    ...(plugins ? { plugins } : {}),
    ...(overrides ? { overrides } : {}),
  };
}

function extractExecutionTypes(executionBlock: Record<string, any> | undefined): string[] {
  if (!executionBlock || typeof executionBlock !== 'object') return [];
  const types: string[] = [];
  for (const value of Object.values(executionBlock)) {
    if (!value || typeof value !== 'object') continue;
    const t = typeof (value as any).type === 'string' ? String((value as any).type) : '';
    if (t && !types.includes(t)) types.push(t);

    // For composite, include step types as well for better capability summary.
    if (t === 'composite' && Array.isArray((value as any).steps)) {
      for (const step of (value as any).steps) {
        const st = step && typeof step === 'object' && typeof (step as any).type === 'string' ? String((step as any).type) : '';
        if (st && !types.includes(st)) types.push(st);
      }
    }
  }
  return types;
}

function sortCards(
  actions: ActionCard[],
  queries: QueryCard[],
  packs: PackCard[],
  documents: CatalogDocumentEntry[]
): void {
  actions.sort((a, b) => a.protocol.localeCompare(b.protocol) || a.version.localeCompare(b.version) || a.id.localeCompare(b.id));
  queries.sort((a, b) => a.protocol.localeCompare(b.protocol) || a.version.localeCompare(b.version) || a.id.localeCompare(b.id));
  packs.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
  documents.sort((a, b) => a.path.localeCompare(b.path) || a.kind.localeCompare(b.kind) || String(a.id ?? '').localeCompare(String(b.id ?? '')));
}

function protocolDocFingerprint(spec: ProtocolSpec): unknown {
  return {
    schema: spec.schema,
    protocol: spec.meta.protocol,
    version: spec.meta.version,
    actions: Object.keys(spec.actions ?? {}).slice().sort(),
    queries: Object.keys(spec.queries ?? {}).slice().sort(),
    deployments: spec.deployments.map((d) => d.chain).slice().sort(),
  };
}

function packDocFingerprint(pack: Pack): unknown {
  const name = pack.meta?.name ?? pack.name ?? '(unknown-pack)';
  const version = pack.meta?.version ?? pack.version ?? '(unknown-version)';
  return {
    schema: pack.schema,
    name,
    version,
    includes: (pack.includes ?? []).map((i) => `${i.protocol}@${i.version}`).slice().sort(),
    provider_detect: (pack.providers?.detect?.enabled ?? []).map((e) => `${e.kind}:${e.provider}`).slice().sort(),
    plugin_exec: (pack.plugins?.execution?.enabled ?? []).map((e) => e.type).slice().sort(),
  };
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

function uniqStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}
