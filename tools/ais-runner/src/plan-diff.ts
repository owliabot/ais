import { createHash } from 'node:crypto';

export type PlanDiffChange = {
  field: string;
  a?: unknown;
  b?: unknown;
};

export type PlanNodeDiff = {
  id: string;
  changes: PlanDiffChange[];
};

export type PlanDiffResult = {
  kind: 'plan_diff';
  summary: { added: number; removed: number; changed: number };
  added: string[];
  removed: string[];
  changed: PlanNodeDiff[];
};

export function diffPlans(a: any, b: any): PlanDiffResult {
  const aNodes = Array.isArray(a?.nodes) ? a.nodes : [];
  const bNodes = Array.isArray(b?.nodes) ? b.nodes : [];
  const aById = new Map<string, any>();
  const bById = new Map<string, any>();
  for (const n of aNodes) if (n && typeof n.id === 'string') aById.set(n.id, n);
  for (const n of bNodes) if (n && typeof n.id === 'string') bById.set(n.id, n);

  const added: string[] = [];
  const removed: string[] = [];
  const changed: PlanNodeDiff[] = [];

  for (const id of Array.from(bById.keys()).sort()) {
    if (!aById.has(id)) added.push(id);
  }
  for (const id of Array.from(aById.keys()).sort()) {
    if (!bById.has(id)) removed.push(id);
  }

  for (const id of Array.from(aById.keys()).sort()) {
    const an = aById.get(id);
    const bn = bById.get(id);
    if (!bn) continue;
    const changes: PlanDiffChange[] = [];

    if (String(an.chain ?? '') !== String(bn.chain ?? '')) {
      changes.push({ field: 'chain', a: an.chain, b: bn.chain });
    }
    if (String(an.kind ?? '') !== String(bn.kind ?? '')) {
      changes.push({ field: 'kind', a: an.kind, b: bn.kind });
    }

    const aDeps = normalizeStringArray(an.deps);
    const bDeps = normalizeStringArray(bn.deps);
    if (!sameStringArray(aDeps, bDeps)) {
      changes.push({ field: 'deps', a: aDeps, b: bDeps });
    }

    const aWrites = normalizeWrites(an.writes);
    const bWrites = normalizeWrites(bn.writes);
    if (!sameStringArray(aWrites, bWrites)) {
      changes.push({ field: 'writes', a: aWrites, b: bWrites });
    }

    const aExecType = String(an.execution?.type ?? '');
    const bExecType = String(bn.execution?.type ?? '');
    if (aExecType !== bExecType) {
      changes.push({ field: 'execution.type', a: aExecType, b: bExecType });
    } else {
      const aExecHash = stableHash(an.execution);
      const bExecHash = stableHash(bn.execution);
      if (aExecHash !== bExecHash) {
        changes.push({ field: 'execution', a: { hash: aExecHash }, b: { hash: bExecHash } });
      }
    }

    if (changes.length > 0) changed.push({ id, changes });
  }

  return {
    kind: 'plan_diff',
    summary: { added: added.length, removed: removed.length, changed: changed.length },
    added,
    removed,
    changed,
  };
}

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter((x): x is string => typeof x === 'string' && x.length > 0)
    .slice()
    .sort();
}

function normalizeWrites(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const out: string[] = [];
  for (const w of value) {
    if (!w || typeof w !== 'object' || Array.isArray(w)) continue;
    const rec = w as Record<string, unknown>;
    const path = typeof rec.path === 'string' ? rec.path : '';
    if (!path) continue;
    const mode = typeof rec.mode === 'string' ? rec.mode : 'set';
    out.push(`${path}:${mode}`);
  }
  return out.sort();
}

function sameStringArray(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function stableHash(value: unknown): string {
  const text = JSON.stringify(sortKeysDeep(value));
  return createHash('sha256').update(text).digest('hex');
}

function sortKeysDeep(value: any): any {
  if (Array.isArray(value)) return value.map((v) => sortKeysDeep(v));
  if (!value || typeof value !== 'object') return value;
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) return value;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) out[key] = sortKeysDeep(value[key]);
  return out;
}

