/**
 * Workspace validation - cross-file reference checks (workflow → pack → protocol)
 *
 * This module is intentionally filesystem-agnostic: the CLI/loader provides
 * `{ path, document }` pairs, and this validator only checks relationships.
 */
import type { Pack, ProtocolSpec, Workflow } from '../schema/index.js';
import type { ValueRef } from '../schema/common.js';
import { isCoreExecutionType } from '../schema/index.js';
import { parseProtocolRef } from '../resolver/reference.js';

export type WorkspaceIssueSeverity = 'error' | 'warning' | 'info';

export interface WorkspaceIssue {
  path: string;
  severity: WorkspaceIssueSeverity;
  message: string;
  field_path?: string;
  reference?: string;
  related_path?: string;
}

export interface WorkspaceDocuments {
  protocols: Array<{ path: string; document: ProtocolSpec }>;
  packs: Array<{ path: string; document: Pack }>;
  workflows: Array<{ path: string; document: Workflow }>;
}

export function validateWorkspaceReferences(docs: WorkspaceDocuments): WorkspaceIssue[] {
  const issues: WorkspaceIssue[] = [];

  const protocolById = new Map<string, Array<{ path: string; document: ProtocolSpec }>>();
  const protocolByKey = new Map<string, { path: string; document: ProtocolSpec }>();
  for (const p of docs.protocols) {
    const id = p.document.meta.protocol;
    const version = p.document.meta.version;
    const key = `${id}@${version}`;
    protocolById.set(id, [...(protocolById.get(id) ?? []), p]);
    if (protocolByKey.has(key)) {
      issues.push({
        path: p.path,
        severity: 'error',
        message: `Duplicate protocol version in workspace: ${key}`,
        field_path: 'meta',
        reference: key,
        related_path: protocolByKey.get(key)!.path,
      });
    } else {
      protocolByKey.set(key, p);
    }
  }

  // Without a version-aware resolver context, multiple versions of the same protocol in one workspace are ambiguous.
  for (const [id, arr] of protocolById) {
    const versions = Array.from(new Set(arr.map((p) => p.document.meta.version)));
    if (versions.length > 1) {
      for (const p of arr) {
        issues.push({
          path: p.path,
          severity: 'error',
          message: `Multiple versions of protocol "${id}" in the same workspace: ${versions.join(', ')}`,
          field_path: 'meta.version',
          reference: id,
        });
      }
    }
  }

  const packByName = new Map<string, Array<{ path: string; document: Pack; name: string; version: string }>>();
  const packByKey = new Map<string, { path: string; document: Pack; name: string; version: string }>();
  for (const p of docs.packs) {
    const id = getPackIdentity(p.document);
    if (!id) {
      issues.push({
        path: p.path,
        severity: 'error',
        message: 'Pack must have name+version (either in meta or top-level)',
        field_path: 'meta|name/version',
      });
      continue;
    }
    const key = `${id.name}@${id.version}`;
    const rec = { path: p.path, document: p.document, ...id };
    packByName.set(id.name, [...(packByName.get(id.name) ?? []), rec]);
    if (packByKey.has(key)) {
      issues.push({
        path: p.path,
        severity: 'error',
        message: `Duplicate pack version in workspace: ${key}`,
        field_path: 'meta',
        reference: key,
        related_path: packByKey.get(key)!.path,
      });
    } else {
      packByKey.set(key, rec);
    }
  }

  // Validate pack includes → protocols
  for (const p of packByKey.values()) {
    for (let i = 0; i < p.document.includes.length; i++) {
      const inc = p.document.includes[i]!;
      const key = `${inc.protocol}@${inc.version}`;
      const found = protocolByKey.get(key);
      if (!found) {
        const candidates = protocolById.get(inc.protocol) ?? [];
        const candidateVersions = Array.from(new Set(candidates.map((c) => c.document.meta.version)));
        issues.push({
          path: p.path,
          severity: 'error',
          message:
            candidateVersions.length > 0
              ? `Pack includes ${key}, but workspace has ${inc.protocol} versions: ${candidateVersions.join(', ')}`
              : `Pack includes missing protocol: ${key}`,
          field_path: `includes[${i}]`,
          reference: key,
        });
        continue;
      }

      // Pack provider/plugin allowlists are part of the capabilities boundary:
      // If a pack includes a protocol, it should not include features it explicitly disables.
      issues.push(...validatePackAgainstProtocol(p.path, p.document, found.path, found.document));

      if (inc.chain_scope && inc.chain_scope.length > 0) {
        // chain_scope is validated in workflow checks where node.chain is known.
        // Here we only sanity-check that chain_scope has unique entries.
        const uniq = new Set(inc.chain_scope);
        if (uniq.size !== inc.chain_scope.length) {
          issues.push({
            path: p.path,
            severity: 'warning',
            message: `Pack include ${key} has duplicate chain_scope entries`,
            field_path: `includes[${i}].chain_scope`,
            reference: key,
            related_path: found.path,
          });
        }
      }
    }
  }

  // Validate workflows → pack → protocols/actions/queries
  for (const wf of docs.workflows) {
    const requiredPack = wf.document.requires_pack
      ? `${wf.document.requires_pack.name}@${wf.document.requires_pack.version}`
      : null;
    const pack = requiredPack ? packByKey.get(requiredPack) : null;

    if (requiredPack && !pack) {
      const candidates = packByName.get(wf.document.requires_pack!.name) ?? [];
      const versions = Array.from(new Set(candidates.map((c) => c.version)));
      issues.push({
        path: wf.path,
        severity: 'error',
        message:
          versions.length > 0
            ? `Workflow requires pack ${requiredPack}, but workspace has versions: ${versions.join(', ')}`
            : `Workflow requires missing pack: ${requiredPack}`,
        field_path: 'requires_pack',
        reference: requiredPack,
      });
    }

    const packIncludes = pack ? buildPackIncludeIndex(pack.document) : null;
    const workflowDefaultChain = wf.document.default_chain;

    for (let i = 0; i < wf.document.nodes.length; i++) {
      const node = wf.document.nodes[i]!;
      const protoRef = (node as any).protocol;
      const { protocol, version } = parseProtocolRef(String(protoRef ?? ''));
      const key = `${protocol}@${version ?? ''}`;

      if (!version) {
        issues.push({
          path: wf.path,
          severity: 'error',
          message: `Node protocol must include version: ${protocol}@<version>`,
          field_path: `nodes[${i}].protocol`,
          reference: String(protoRef ?? ''),
        });
        continue;
      }

      const spec = protocolByKey.get(`${protocol}@${version}`);
      if (!spec) {
        const candidates = protocolById.get(protocol) ?? [];
        const candidateVersions = Array.from(new Set(candidates.map((c) => c.document.meta.version)));
        issues.push({
          path: wf.path,
          severity: 'error',
          message:
            candidateVersions.length > 0
              ? `Node references ${protocol}@${version}, but workspace has versions: ${candidateVersions.join(', ')}`
              : `Node references missing protocol: ${protocol}@${version}`,
          field_path: `nodes[${i}].protocol`,
          reference: `${protocol}@${version}`,
        });
        continue;
      }

      if (pack && packIncludes) {
        if (!packIncludes.includes.has(`${protocol}@${version}`)) {
          issues.push({
            path: wf.path,
            severity: 'error',
            message: `Workflow requires pack ${requiredPack}, but node uses protocol not included: ${protocol}@${version}`,
            field_path: `nodes[${i}].protocol`,
            reference: `${protocol}@${version}`,
            related_path: pack.path,
          });
        } else {
          const chain = node.chain ?? workflowDefaultChain;
          const include = packIncludes.byProtocol.get(`${protocol}@${version}`);
          if (chain && include?.chain_scope && include.chain_scope.length > 0 && !include.chain_scope.includes(chain)) {
            issues.push({
              path: wf.path,
              severity: 'error',
              message: `Node chain "${chain}" not allowed by pack chain_scope for ${protocol}@${version}`,
              field_path: `nodes[${i}].chain`,
              reference: chain,
              related_path: pack.path,
            });
          }
        }
      }

      if (node.type === 'action_ref') {
        if (!node.action) {
          issues.push({
            path: wf.path,
            severity: 'error',
            message: 'action_ref node must set nodes[].action',
            field_path: `nodes[${i}].action`,
          });
        } else if (!spec.document.actions[node.action]) {
          issues.push({
            path: wf.path,
            severity: 'error',
            message: `Action not found: ${protocol}@${version}/${node.action}`,
            field_path: `nodes[${i}].action`,
            reference: node.action,
            related_path: spec.path,
          });
        }
      }

      if (node.type === 'query_ref') {
        if (!node.query) {
          issues.push({
            path: wf.path,
            severity: 'error',
            message: 'query_ref node must set nodes[].query',
            field_path: `nodes[${i}].query`,
          });
        } else if (!spec.document.queries || !spec.document.queries[node.query]) {
          issues.push({
            path: wf.path,
            severity: 'error',
            message: `Query not found: ${protocol}@${version}/${node.query}`,
            field_path: `nodes[${i}].query`,
            reference: node.query,
            related_path: spec.path,
          });
        }
      }

      // Detect provider checks are pack-scoped: if a workflow requires a pack, it should not use detect kinds
      // that the pack didn't enable.
      if (pack && packIncludes) {
        const detects = collectDetectsFromWorkflowNode(node);
        for (const d of detects) {
          if (!packIncludes.detectProviders) continue;
          const enabled = packIncludes.detectProviders.get(d.kind) ?? [];
          const ok = d.provider ? enabled.includes(d.provider) : enabled.length > 0;
          if (!ok) {
            issues.push({
              path: wf.path,
              severity: 'error',
              message: d.provider
                ? `Detect(kind=${d.kind}, provider=${d.provider}) used by node "${node.id}" but not enabled in pack ${requiredPack}`
                : `Detect(kind=${d.kind}) used by node "${node.id}" but no provider enabled in pack ${requiredPack}`,
              field_path: `nodes[${i}].${d.field_path}`,
              reference: d.provider ? `${d.kind}/${d.provider}` : d.kind,
              related_path: pack.path,
            });
          }
        }
      }
    }
  }

  return issues;
}

function getPackIdentity(pack: Pack): { name: string; version: string } | null {
  const meta = pack.meta;
  const name = meta?.name ?? pack.name;
  const version = meta?.version ?? pack.version;
  if (!name || !version) return null;
  return { name, version };
}

function buildPackIncludeIndex(pack: Pack): {
  includes: Set<string>;
  byProtocol: Map<string, Pack['includes'][number]>;
  detectProviders: Map<string, string[]> | null;
} {
  const includes = new Set<string>();
  const byProtocol = new Map<string, Pack['includes'][number]>();

  for (const inc of pack.includes) {
    const key = `${inc.protocol}@${inc.version}`;
    includes.add(key);
    if (!byProtocol.has(key)) byProtocol.set(key, inc);
  }

  const detectProviders = pack.providers?.detect?.enabled
    ? buildDetectProvidersIndex(pack.providers.detect.enabled)
    : null;

  return { includes, byProtocol, detectProviders };
}

function buildDetectProvidersIndex(enabled: Array<{ kind: string; provider: string }>): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const e of enabled) {
    const prev = map.get(e.kind) ?? [];
    if (!prev.includes(e.provider)) prev.push(e.provider);
    map.set(e.kind, prev);
  }
  return map;
}

function validatePackAgainstProtocol(
  packPath: string,
  pack: Pack,
  protocolPath: string,
  protocol: ProtocolSpec
): WorkspaceIssue[] {
  const issues: WorkspaceIssue[] = [];

  const allowedExecutionTypes = new Set<string>(
    pack.plugins?.execution?.enabled
      ?.map((e: any) => (typeof e?.type === 'string' ? e.type : null))
      .filter((t: unknown) => typeof t === 'string') ?? []
  );
  if (allowedExecutionTypes.size > 0) {
    for (const t of collectPluginExecutionTypes(protocol)) {
      if (!allowedExecutionTypes.has(t)) {
        issues.push({
          path: packPath,
          severity: 'error',
          message: `Pack blocks plugin execution type "${t}" used by included protocol ${protocol.meta.protocol}@${protocol.meta.version}`,
          field_path: 'plugins.execution.enabled',
          reference: t,
          related_path: protocolPath,
        });
      }
    }
  }

  const allowedDetectProviders = new Set<string>(
    pack.providers?.detect?.enabled
      ?.map((e: any) =>
        typeof e?.kind === 'string' && typeof e?.provider === 'string' ? `${e.kind}:${e.provider}` : null
      )
      .filter((x: unknown) => typeof x === 'string') ?? []
  );
  if (allowedDetectProviders.size > 0) {
    for (const d of collectDetectProviderRefs(protocol)) {
      const key = `${d.kind}:${d.provider}`;
      if (!allowedDetectProviders.has(key)) {
        issues.push({
          path: packPath,
          severity: 'error',
          message: `Pack blocks detect provider "${key}" referenced by included protocol ${protocol.meta.protocol}@${protocol.meta.version}`,
          field_path: 'providers.detect.enabled',
          reference: key,
          related_path: protocolPath,
        });
      }
    }
  }

  return issues;
}

function collectPluginExecutionTypes(protocol: ProtocolSpec): string[] {
  const out: string[] = [];

  const walkExecution = (ex: unknown): void => {
    if (!ex || typeof ex !== 'object') return;
    const t = (ex as any).type;
    if (typeof t !== 'string') return;

    if (t === 'composite') {
      const steps = (ex as any).steps;
      if (Array.isArray(steps)) {
        for (const s of steps) walkExecution((s as any)?.execution);
      }
      return;
    }

    if (!isCoreExecutionType(t)) out.push(t);
  };

  for (const action of Object.values(protocol.actions)) {
    for (const ex of Object.values(action.execution ?? {})) walkExecution(ex);
  }
  for (const query of Object.values(protocol.queries ?? {})) {
    for (const ex of Object.values(query.execution ?? {})) walkExecution(ex);
  }

  return Array.from(new Set(out));
}

function collectDetectProviderRefs(protocol: ProtocolSpec): Array<{ kind: string; provider: string }> {
  const out: Array<{ kind: string; provider: string }> = [];
  const seen = new Set<unknown>();

  const walk = (v: unknown): void => {
    if (!v || typeof v !== 'object') return;
    if (seen.has(v)) return;
    seen.add(v);

    if (isDetectValueRef(v)) {
      const detect = (v as any).detect;
      if (detect && typeof detect === 'object' && typeof detect.kind === 'string' && typeof detect.provider === 'string') {
        out.push({ kind: detect.kind, provider: detect.provider });
      }
      walk(detect);
      return;
    }

    if (Array.isArray(v)) {
      for (const item of v) walk(item);
      return;
    }

    for (const vv of Object.values(v as Record<string, unknown>)) walk(vv);
  };

  walk(protocol);
  return out;
}

function isDetectValueRef(v: unknown): boolean {
  if (!v || typeof v !== 'object') return false;
  const keys = Object.keys(v as Record<string, unknown>);
  return keys.length === 1 && keys[0] === 'detect';
}

function collectDetectsFromWorkflowNode(node: Workflow['nodes'][number]): Array<{
  kind: string;
  provider?: string;
  field_path: string;
}> {
  const detects: Array<{ kind: string; provider?: string; field_path: string }> = [];

  const visitValueRef = (v: ValueRef | undefined, fieldPath: string): void => {
    if (!v) return;
    if (typeof v !== 'object' || v === null) return;
    const rec = v as any;
    if (rec.detect && typeof rec.detect === 'object') {
      const kind = rec.detect.kind;
      if (typeof kind === 'string') {
        const provider = typeof rec.detect.provider === 'string' ? rec.detect.provider : undefined;
        detects.push({ kind, provider, field_path: fieldPath });
      }
      return;
    }
    if (rec.object && typeof rec.object === 'object') {
      for (const [k, child] of Object.entries(rec.object)) visitValueRef(child as ValueRef, `${fieldPath}.object.${k}`);
      return;
    }
    if (rec.array && Array.isArray(rec.array)) {
      for (let i = 0; i < rec.array.length; i++) visitValueRef(rec.array[i] as ValueRef, `${fieldPath}.array[${i}]`);
      return;
    }
  };

  if (node.args) {
    for (const [k, v] of Object.entries(node.args)) visitValueRef(v as ValueRef, `args.${k}`);
  }
  if (node.condition) visitValueRef(node.condition as ValueRef, 'condition');
  if (node.until) visitValueRef(node.until as ValueRef, 'until');
  if (node.calculated_overrides) {
    for (const [k, ov] of Object.entries(node.calculated_overrides)) {
      visitValueRef((ov as any).expr as ValueRef, `calculated_overrides.${k}.expr`);
    }
  }
  return detects;
}
