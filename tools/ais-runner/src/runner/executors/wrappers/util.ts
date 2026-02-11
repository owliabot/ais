import type { RunnerPack, RunnerPlanNode } from '../../../types.js';

export function classifyIo(node: RunnerPlanNode): 'read' | 'write' {
  if (node.kind === 'query_ref') return 'read';
  const execType = node.execution.type;
  if (execType === 'evm_read' || execType === 'evm_rpc' || execType === 'evm_multiread' || execType === 'solana_read') {
    return 'read';
  }
  return 'write';
}

export function uniqStrings(values: string[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const value of values) {
    if (typeof value !== 'string' || value.length === 0) continue;
    if (seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

export function packMeta(pack: RunnerPack | undefined): { name?: string; version?: string } | undefined {
  if (!pack) return undefined;
  const name = pack.meta?.name ?? pack.name;
  const version = pack.meta?.version ?? pack.version;
  const out: { name?: string; version?: string } = {};
  if (name) out.name = String(name);
  if (version) out.version = String(version);
  return Object.keys(out).length > 0 ? out : undefined;
}

export function policyApprovalsSummary(policy: RunnerPack['policy'] | undefined): unknown {
  if (!policy || !policy.approvals) return undefined;
  const out: {
    auto_execute_max_risk_level?: number;
    require_approval_min_risk_level?: number;
  } = {};
  if (policy.approvals.auto_execute_max_risk_level !== undefined) {
    out.auto_execute_max_risk_level = policy.approvals.auto_execute_max_risk_level;
  }
  if (policy.approvals.require_approval_min_risk_level !== undefined) {
    out.require_approval_min_risk_level = policy.approvals.require_approval_min_risk_level;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

export function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

export function isEvmFailureStatus(status: unknown): boolean {
  if (status === 0 || status === false) return true;
  if (status === 1 || status === true) return false;
  if (typeof status === 'string') {
    const normalized = status.toLowerCase();
    if (normalized === '0x0' || normalized === '0') return true;
    if (normalized === '0x1' || normalized === '1') return false;
  }
  return false;
}

export function topoOrderCalculatedFields(
  calculated: Record<string, { inputs?: string[] }>
): string[] {
  const names = Object.keys(calculated);
  const originalIndex = new Map(names.map((name, index) => [name, index] as const));

  const depsByName = new Map<string, Set<string>>();
  const outgoing = new Map<string, Set<string>>();
  const inDegree = new Map<string, number>();

  for (const name of names) {
    depsByName.set(name, new Set());
    outgoing.set(name, new Set());
    inDegree.set(name, 0);
  }

  for (const [name, def] of Object.entries(calculated)) {
    for (const input of def?.inputs ?? []) {
      if (typeof input !== 'string') continue;
      const dep = extractCalculatedDep(input);
      if (!dep) continue;
      if (!depsByName.has(name) || !depsByName.has(dep)) continue;
      depsByName.get(name)?.add(dep);
    }
  }

  for (const [name, deps] of depsByName.entries()) {
    for (const dep of deps) {
      outgoing.get(dep)?.add(name);
      inDegree.set(name, (inDegree.get(name) ?? 0) + 1);
    }
  }

  const available: string[] = [];
  for (const name of names) {
    if ((inDegree.get(name) ?? 0) === 0) available.push(name);
  }

  const sortAvailable = () =>
    available.sort((left, right) => {
      const leftIndex = originalIndex.get(left) ?? 0;
      const rightIndex = originalIndex.get(right) ?? 0;
      if (leftIndex !== rightIndex) return leftIndex - rightIndex;
      return left.localeCompare(right);
    });
  sortAvailable();

  const ordered: string[] = [];
  while (available.length > 0) {
    const name = available.shift();
    if (!name) break;
    ordered.push(name);
    for (const next of outgoing.get(name) ?? []) {
      const deg = (inDegree.get(next) ?? 0) - 1;
      inDegree.set(next, deg);
      if (deg === 0) available.push(next);
    }
    sortAvailable();
  }

  if (ordered.length !== names.length) {
    return names.slice().sort((left, right) => (originalIndex.get(left) ?? 0) - (originalIndex.get(right) ?? 0));
  }

  return ordered;
}

function extractCalculatedDep(input: string): string | null {
  if (!input.startsWith('calculated.')) return null;
  const rest = input.slice('calculated.'.length);
  const first = rest.split('.', 1)[0];
  return first && /^[A-Za-z_][A-Za-z0-9_]*$/.test(first) ? first : null;
}
