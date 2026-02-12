import { createHash } from 'node:crypto';
import { z } from 'zod';
import type { ExecutionPlanNode } from '../execution/index.js';
import { ExtensionsSchema } from '../schema/common.js';

export const ConfirmationSummarySchemaVersion = 'ais-confirmation-summary/0.0.1' as const;

export const ConfirmationNodeSummarySchema = z
  .object({
    node_id: z.string().min(1),
    workflow_node_id: z.string().min(1).optional(),
    action_ref: z.string().min(1).optional(),
    action_key: z.string().min(1).optional(),
    chain: z.string().min(1),
    execution_type: z.string().min(1),
    kind: z.string().min(1).optional(),
    writes: z.array(z.object({ path: z.string().min(1), mode: z.string().min(1).optional() }).strict()).optional(),
  })
  .strict();

export const ConfirmationRiskSummarySchema = z
  .object({
    risk_level: z.number().int().min(1).max(5).optional(),
    risk_tags: z.array(z.string().min(1)).optional(),
    require_approval_min_risk_level: z.number().int().min(1).max(5).optional(),
  })
  .strict();

export const ConfirmationPreviewSummarySchema = z.record(z.unknown());

export const ConfirmationSummarySchema = z
  .object({
    schema: z.literal(ConfirmationSummarySchemaVersion),
    hash: z.string().min(1),
    title: z.string().min(1),
    summary: z.string().min(1),
    node: ConfirmationNodeSummarySchema,
    hit_reasons: z.array(z.string().min(1)).optional(),
    risk: ConfirmationRiskSummarySchema.optional(),
    preview: ConfirmationPreviewSummarySchema.optional(),
    gate: z.record(z.unknown()).optional(),
    extensions: ExtensionsSchema.optional(),
  })
  .strict();

export type ConfirmationSummary = z.infer<typeof ConfirmationSummarySchema>;

export function summarizeNeedUserConfirm(args: {
  node: ExecutionPlanNode;
  reason: string;
  details?: unknown;
}): ConfirmationSummary {
  const { node, reason } = args;
  const details = asRecord(args.details);

  const workflow_node_id =
    (details && asString(details.workflow_node_id)) ??
    (typeof node.source?.node_id === 'string' && node.source.node_id.length > 0 ? node.source.node_id : undefined);

  const action_ref =
    (details && asString(details.action_ref)) ??
    (typeof node.source?.protocol === 'string' &&
    typeof node.source?.action === 'string' &&
    node.source.protocol.length > 0 &&
    node.source.action.length > 0
      ? `${node.source.protocol}/${node.source.action}`
      : undefined);

  const action_key =
    (details && asString(details.action_key)) ??
    (typeof node.source?.protocol === 'string' && typeof node.source?.action === 'string'
      ? buildActionKey(node.source.protocol, node.source.action)
      : undefined);

  const kind = (details && asString(details.kind)) ?? inferNeedConfirmKind(reason, details);
  const hit_reasons = collectHitReasons(reason, details);

  const gate = details ? asRecord(details.gate) : null;
  const gateDetails = gate ? asRecord(gate.details) : null;
  const gateInput = gateDetails ? asRecord(gateDetails.gate_input) : null;

  const preview =
    (details ? details.preview : undefined) ??
    (gateInput ? gateInput.preview : undefined);

  const riskFromDetails = details ? asRecord(details.risk) : null;
  const riskLevel =
    (gateInput && typeof gateInput.risk_level === 'number' ? gateInput.risk_level : undefined) ??
    (riskFromDetails && typeof riskFromDetails.risk_level === 'number' ? riskFromDetails.risk_level : undefined);
  const riskTags =
    (gateInput && Array.isArray(gateInput.risk_tags) ? gateInput.risk_tags : undefined) ??
    (riskFromDetails && Array.isArray(riskFromDetails.risk_tags) ? riskFromDetails.risk_tags : undefined);
  const threshold =
    (riskFromDetails && typeof riskFromDetails.require_approval_min_risk_level === 'number'
      ? riskFromDetails.require_approval_min_risk_level
      : undefined) ??
    (gateDetails && typeof (gateDetails as any).require_approval_min_risk_level === 'number'
      ? (gateDetails as any).require_approval_min_risk_level
      : undefined);

  const nodeSummary: z.infer<typeof ConfirmationNodeSummarySchema> = {
    node_id: node.id,
    ...(workflow_node_id ? { workflow_node_id } : {}),
    ...(action_ref ? { action_ref } : {}),
    ...(action_key ? { action_key } : {}),
    chain: node.chain,
    execution_type: node.execution.type,
    ...(kind ? { kind } : {}),
    ...(Array.isArray(node.writes) && node.writes.length > 0
      ? { writes: node.writes.map((w) => ({ path: w.path, mode: (w as any).mode })) }
      : {}),
  };

  const risk: z.infer<typeof ConfirmationRiskSummarySchema> | undefined =
    riskLevel !== undefined || (Array.isArray(riskTags) && riskTags.length > 0) || threshold !== undefined
      ? {
          ...(riskLevel !== undefined ? { risk_level: riskLevel } : {}),
          ...(Array.isArray(riskTags) && riskTags.length > 0
            ? { risk_tags: uniqStrings(riskTags.map(String)).slice().sort() }
            : {}),
          ...(threshold !== undefined ? { require_approval_min_risk_level: threshold } : {}),
        }
      : undefined;

  const title = buildTitle(kind);
  const summary = buildSummary({
    node: nodeSummary,
    risk,
    hit_reasons,
    preview,
  });

  const contentForHash = {
    schema: ConfirmationSummarySchemaVersion,
    title,
    summary,
    node: nodeSummary,
    hit_reasons,
    risk,
    preview,
    gate: gate ?? undefined,
  };
  const hash = sha256Hex(stableJsonStringify(contentForHash));

  const out: ConfirmationSummary = {
    schema: ConfirmationSummarySchemaVersion,
    hash,
    title,
    summary,
    node: nodeSummary,
    ...(hit_reasons.length > 0 ? { hit_reasons } : {}),
    ...(risk ? { risk } : {}),
    ...(preview !== undefined ? { preview: preview as any } : {}),
    ...(gate ? { gate } : {}),
  };

  // Validate for stability (dev-safety; should always pass).
  const parsed = ConfirmationSummarySchema.safeParse(out);
  if (!parsed.success) {
    // Fallback: return minimal stable shape rather than throwing inside engine loop.
    return {
      schema: ConfirmationSummarySchemaVersion,
      hash: sha256Hex(stableJsonStringify({ schema: ConfirmationSummarySchemaVersion, node: nodeSummary, reason })),
      title: '需要确认',
      summary: `reason=${reason}`,
      node: nodeSummary,
      hit_reasons: hit_reasons.length > 0 ? hit_reasons : undefined,
    } as ConfirmationSummary;
  }
  return parsed.data;
}

function buildTitle(kind: string): string {
  if (kind === 'broadcast_gate') return '需要确认：允许广播交易';
  if (kind === 'policy_allowlist') return '需要确认：执行类型不在 allowlist';
  if (kind === 'policy_gate') return '需要确认：策略规则触发';
  if (kind === 'policy_gate_resolution') return '需要确认：策略 gate 无法解析动作';
  return '需要确认';
}

function buildSummary(args: {
  node: z.infer<typeof ConfirmationNodeSummarySchema>;
  risk?: z.infer<typeof ConfirmationRiskSummarySchema>;
  hit_reasons: string[];
  preview: unknown;
}): string {
  const parts: string[] = [];
  parts.push(`chain=${args.node.chain}`);
  if (args.node.action_ref) parts.push(`action=${args.node.action_ref}`);
  parts.push(`exec=${args.node.execution_type}`);
  if (args.risk?.risk_level !== undefined) parts.push(`risk=${args.risk.risk_level}`);
  if (Array.isArray(args.risk?.risk_tags) && args.risk!.risk_tags!.length > 0) {
    parts.push(`tags=${args.risk!.risk_tags!.join(',')}`);
  }
  const preview = asRecord(args.preview);
  if (preview) {
    const pk = asString(preview.kind);
    if (pk) parts.push(`preview=${pk}`);
    const to = asString(preview.to);
    if (to) parts.push(`to=${to}`);
    const fn = asString(preview.function_name);
    if (fn) parts.push(`fn=${fn}`);
    const program = asString(preview.program_id);
    if (program) parts.push(`program=${program}`);
    const instruction = asString(preview.instruction);
    if (instruction) parts.push(`ix=${instruction}`);
  }
  if (args.hit_reasons.length > 0) parts.push(`hits=${args.hit_reasons.join(' | ')}`);
  return parts.join(' ');
}

function inferNeedConfirmKind(reason: string, details: Record<string, unknown> | null): string {
  if (details && asRecord(details.gate)) return 'policy_gate';
  if (reason.includes('broadcast disabled')) return 'broadcast_gate';
  return 'need_user_confirm';
}

function collectHitReasons(reason: string, details: Record<string, unknown> | null): string[] {
  const out: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== 'string' || value.length === 0) return;
    if (!out.includes(value)) out.push(value);
  };
  push(reason);
  if (!details) return out;

  for (const value of asStringArray(details.hit_reasons)) push(value);
  for (const value of asStringArray(details.missing_fields)) push(`missing_field:${value}`);

  const gate = asRecord(details.gate);
  if (gate) {
    push(gate.reason);
    const gateDetails = asRecord(gate.details);
    if (gateDetails) {
      for (const value of asStringArray(gateDetails.approval_reasons)) push(value);
      for (const value of asStringArray(gateDetails.missing_fields)) push(`missing_field:${value}`);
      const violations = Array.isArray(gateDetails.violations) ? gateDetails.violations : [];
      for (const violation of violations) {
        const rec = asRecord(violation);
        if (!rec) continue;
        push(rec.message);
        push(rec.constraint);
      }
    }
  }
  return out;
}

function buildActionKey(protocolRef: string, actionId: string): string {
  const protocol = protocolRef.split('@', 1)[0] ?? protocolRef;
  return `${protocol}.${actionId}`;
}

function stableJsonStringify(value: unknown): string {
  return JSON.stringify(sortKeysDeep(value));
}

function sortKeysDeep(value: any): any {
  if (Array.isArray(value)) return value.map((v) => sortKeysDeep(v));
  if (!value || typeof value !== 'object') return value;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) {
    // never hash volatile fields if present
    if (key === 'created_at' || key === 'ts') continue;
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
  for (const v of values) {
    const value = String(v ?? '').trim();
    if (!value || seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
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

