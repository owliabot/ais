import type { RunnerSdkModule, RunnerWorkflow, RunnerWorkspaceDocuments } from '../../types.js';

export function collectRelevantWorkspacePaths(
  sdk: RunnerSdkModule,
  docs: RunnerWorkspaceDocuments,
  workflowPath: string,
  workflow: RunnerWorkflow
): Set<string> {
  const out = new Set<string>();
  out.add(workflowPath);

  const req = workflow.requires_pack;
  if (req && typeof req === 'object') {
    const wantName = String(req.name ?? '');
    const wantVer = String(req.version ?? '');
    for (const p of docs.packs) {
      const meta = p.document?.meta;
      const name = meta?.name ? String(meta.name) : '';
      const version = meta?.version ? String(meta.version) : '';
      if (name === wantName && version === wantVer) {
        out.add(String(p.path));
        break;
      }
    }
  }

  for (const n of workflow.nodes) {
    const protocolRef = n.protocol;
    if (typeof protocolRef !== 'string') continue;
    const parsed = sdk.parseProtocolRef(protocolRef);
    const protocol = String(parsed?.protocol ?? '');
    const version = String(parsed?.version ?? '');
    if (!protocol || !version) continue;
    for (const pr of docs.protocols) {
      const meta = pr.document?.meta;
      if (String(meta?.protocol ?? '') === protocol && String(meta?.version ?? '') === version) {
        out.add(String(pr.path));
        break;
      }
    }
  }

  return out;
}

export function findProtocolPathByRef(
  sdk: RunnerSdkModule,
  docs: RunnerWorkspaceDocuments,
  protocolRef: string
): string | null {
  const parsed = sdk.parseProtocolRef(protocolRef);
  const protocol = String(parsed?.protocol ?? '');
  const version = String(parsed?.version ?? '');
  if (!protocol || !version) return null;
  for (const pr of docs.protocols) {
    const meta = pr.document?.meta;
    if (String(meta?.protocol ?? '') === protocol && String(meta?.version ?? '') === version) {
      return String(pr.path);
    }
  }
  return null;
}

export function findRequiredPackDocument(
  workflow: RunnerWorkflow,
  docs: RunnerWorkspaceDocuments
): RunnerWorkspaceDocuments['packs'][number] | null {
  const req = workflow.requires_pack;
  if (!req || typeof req !== 'object') return null;
  const wantName = String(req.name ?? '');
  const wantVer = String(req.version ?? '');
  if (!wantName || !wantVer) return null;

  for (const p of docs.packs) {
    const doc = p.document;
    const meta = doc?.meta;
    const name = meta?.name ? String(meta.name) : doc?.name ? String(doc.name) : '';
    const version = meta?.version ? String(meta.version) : doc?.version ? String(doc.version) : '';
    if (name === wantName && version === wantVer) return p;
  }
  return null;
}
