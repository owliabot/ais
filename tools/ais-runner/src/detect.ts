type DetectResolver = { resolve(detect: any, ctx: any): unknown | Promise<unknown> };

type ProviderEntry = {
  kind: string;
  provider: string;
  priority: number;
  resolve: (detect: any, ctx: any) => unknown | Promise<unknown>;
};

export function createRunnerDetectResolver(args: {
  sdk: any;
  workflow?: any;
  workspaceDocs?: any;
}): DetectResolver | undefined {
  const { sdk, workflow, workspaceDocs } = args;
  if (!sdk) return undefined;

  const byKind = new Map<string, ProviderEntry[]>();
  const byKey = new Map<string, ProviderEntry>();

  const register = (entry: ProviderEntry) => {
    const key = `${entry.kind}:${entry.provider}`;
    byKey.set(key, entry);
    const arr = byKind.get(entry.kind) ?? [];
    arr.push(entry);
    byKind.set(entry.kind, arr);
  };

  // Minimal built-ins (deterministic): picks first candidate.
  register({
    kind: 'best_quote',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk, detect),
  });
  register({
    kind: 'best_path',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk, detect),
  });
  register({
    kind: 'protocol_specific',
    provider: 'builtin-first',
    priority: -100,
    resolve: (detect) => firstCandidateOrThrow(sdk, detect),
  });

  // Pack-enabled detect providers (if running a workflow with requires_pack).
  const pack = workflow && workspaceDocs ? findRequiredPack(workflow, workspaceDocs) : null;
  const enabled = pack?.document?.providers?.detect?.enabled;
  if (Array.isArray(enabled)) {
    for (const e of enabled) {
      const kind = String(e?.kind ?? '');
      const provider = String(e?.provider ?? '');
      if (!kind || !provider) continue;
      const priority = typeof e?.priority === 'number' ? e.priority : 0;
      const providerCandidates = Array.isArray(e?.candidates) ? e.candidates : undefined;
      register({
        kind,
        provider,
        priority,
        resolve: (detect) => {
          const cs = Array.isArray(detect?.candidates) && detect.candidates.length > 0 ? detect.candidates : providerCandidates;
          if (!Array.isArray(cs) || cs.length === 0) {
            throw new sdk.ValueRefEvalError(`Detect kind "${String(detect?.kind ?? kind)}" requires non-empty candidates`);
          }
          return cs[0];
        },
      });
    }
  }

  return {
    resolve(detect: any, ctx: any) {
      const kind = String(detect?.kind ?? '');
      const provider = detect?.provider ? String(detect.provider) : null;
      if (!kind) throw new sdk.ValueRefEvalError('Detect.kind is required');

      if (provider) {
        const p = byKey.get(`${kind}:${provider}`);
        if (!p) throw new sdk.ValueRefEvalError(`Detect provider not found: kind=${kind} provider=${provider}`);
        return p.resolve(detect, ctx);
      }

      const candidates = byKind.get(kind) ?? [];
      if (candidates.length === 0) {
        throw new sdk.ValueRefEvalError(`Detect kind "${kind}" unsupported (no providers registered)`);
      }

      // When provider is omitted, pick highest priority. This keeps workflows
      // pack-driven and deterministic.
      const best = candidates
        .slice()
        .sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0) || a.provider.localeCompare(b.provider))[0]!;
      return best.resolve(detect, ctx);
    },
  };
}

function firstCandidateOrThrow(sdk: any, detect: any): unknown {
  const cs = detect?.candidates;
  if (!Array.isArray(cs) || cs.length === 0) {
    throw new sdk.ValueRefEvalError('Detect requires non-empty candidates');
  }
  return cs[0];
}

function findRequiredPack(workflow: any, workspaceDocs: any): any | null {
  const req = workflow?.requires_pack;
  if (!req || typeof req !== 'object') return null;
  const wantName = String((req as any).name ?? '');
  const wantVer = String((req as any).version ?? '');
  if (!wantName || !wantVer) return null;

  for (const p of workspaceDocs.packs ?? []) {
    const doc = p?.document;
    const meta = doc?.meta;
    const name = meta?.name ? String(meta.name) : doc?.name ? String(doc.name) : '';
    const version = meta?.version ? String(meta.version) : doc?.version ? String(doc.version) : '';
    if (name === wantName && version === wantVer) return p;
  }
  return null;
}

