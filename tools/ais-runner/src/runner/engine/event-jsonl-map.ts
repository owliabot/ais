import type { RunnerEngineEvent, RunnerPlanNode } from '../../types.js';

export function normalizeEventForJsonl(ev: RunnerEngineEvent): unknown {
  if (ev.type !== 'need_user_confirm') return ev;
  const details = asRecord(ev.details) ?? {};
  const node = ev.node;
  const gate = asRecord(details.gate);
  const gateDetails = asRecord(gate?.details);
  const normalizedDetails: Record<string, unknown> = {
    ...details,
    node_id: node.id,
    workflow_node_id: asString(node.source?.node_id) ?? node.id,
    action_ref: buildActionRef(node),
    chain: node.chain,
    execution_type: node.execution.type,
    hit_reasons: collectHitReasons(ev.reason, details),
    pack_summary: summarizePack(details, gateDetails),
    policy_summary: summarizePolicy(details, gateDetails),
  };

  if (!normalizedDetails.kind) {
    normalizedDetails.kind = inferNeedConfirmKind(ev.reason, details);
  }

  return {
    ...ev,
    details: normalizedDetails,
  };
}

function buildActionRef(node: RunnerPlanNode): string | undefined {
  const protocol = asString(node.source?.protocol);
  const action = asString(node.source?.action);
  if (!protocol || !action) return undefined;
  return `${protocol}/${action}`;
}

function inferNeedConfirmKind(reason: string, details: Record<string, unknown>): string {
  if (asRecord(details.gate)) return 'policy_gate';
  if (reason.includes('broadcast disabled')) return 'broadcast_gate';
  return 'need_user_confirm';
}

function collectHitReasons(reason: string, details: Record<string, unknown>): string[] {
  const out: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== 'string' || value.length === 0) return;
    if (!out.includes(value)) out.push(value);
  };

  push(reason);

  for (const value of asStringArray(details.hit_reasons)) push(value);
  for (const value of asStringArray(details.missing_fields).map((field) => `missing_field:${field}`)) push(value);

  const gate = asRecord(details.gate);
  if (gate) {
    push(gate.reason);
    const gateDetails = asRecord(gate.details);
    if (gateDetails) {
      for (const value of asStringArray(gateDetails.approval_reasons)) push(value);
      for (const field of asStringArray(gateDetails.missing_fields)) push(`missing_field:${field}`);
      const violations = Array.isArray(gateDetails.violations) ? gateDetails.violations : [];
      for (const violation of violations) {
        const rec = asRecord(violation);
        if (!rec) continue;
        push(asString(rec.message) ?? asString(rec.constraint));
      }
    }
  }

  return out;
}

function summarizePack(
  details: Record<string, unknown>,
  gateDetails: Record<string, unknown> | null
): Record<string, unknown> | undefined {
  const out: Record<string, unknown> = {};
  const pack = asRecord(details.pack) ?? asRecord(gateDetails?.pack);
  if (pack) {
    const name = asString(pack.name);
    const version = asString(pack.version);
    if (name) out.name = name;
    if (version) out.version = version;
  }
  const protocol = asString(details.protocol) ?? asString(gateDetails?.protocol);
  const action = asString(details.action) ?? asString(gateDetails?.action);
  if (protocol) out.protocol = protocol;
  if (action) out.action = action;
  return Object.keys(out).length > 0 ? out : undefined;
}

function summarizePolicy(
  details: Record<string, unknown>,
  gateDetails: Record<string, unknown> | null
): Record<string, unknown> | undefined {
  const out: Record<string, unknown> = {};
  const policy = asRecord(details.policy) ?? asRecord(gateDetails?.policy);
  if (policy) {
    const mode = asString(policy.mode);
    if (mode) out.mode = mode;
    const strict = policy.strict;
    if (typeof strict === 'boolean') out.strict = strict;
  }
  const risk = asRecord(details.risk) ?? asRecord(gateDetails?.risk);
  if (risk) {
    if (typeof risk.risk_level === 'number') out.risk_level = risk.risk_level;
    const threshold = risk.require_approval_min_risk_level;
    if (typeof threshold === 'number') out.require_approval_min_risk_level = threshold;
  }
  const missingFields = asStringArray(details.missing_fields);
  if (missingFields.length > 0) out.missing_fields = missingFields;
  return Object.keys(out).length > 0 ? out : undefined;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0);
}
