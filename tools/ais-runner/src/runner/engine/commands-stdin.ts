import { createHash } from 'node:crypto';
import type { RunnerContext, RunnerEngineEvent, RunnerPlan, RunnerSdkModule } from '../../types.js';

type ConsumeCommandsArgs = {
  sdk: RunnerSdkModule;
  plan: RunnerPlan;
  context: RunnerContext;
  lines: AsyncIterable<string>;
  seenCommandIds: Set<string>;
  pausedNodeIds: Set<string>;
};

type ParsedRunnerCommand = {
  id: string;
  ts: string;
  kind: 'apply_patches' | 'user_confirm' | 'select_provider' | 'cancel';
  payload: any;
};

export type ConsumedCommandsResult = {
  events: RunnerEngineEvent[];
  accepted_count: number;
  rejected_count: number;
  rerun_requested: boolean;
  cancel_requested: boolean;
};

export async function consumeCommandLines(args: ConsumeCommandsArgs): Promise<ConsumedCommandsResult> {
  const { sdk, plan, context, lines, seenCommandIds, pausedNodeIds } = args;
  const events: RunnerEngineEvent[] = [];
  let accepted_count = 0;
  let rejected_count = 0;
  let rerun_requested = false;
  let cancel_requested = false;

  for await (const rawLine of lines) {
    const line = String(rawLine).trim();
    if (!line) continue;

    let payload: unknown;
    try {
      payload = sdk.parseAisJson(line);
    } catch (error) {
      rejected_count++;
      events.push({
        type: 'command_rejected',
        reason: 'invalid command jsonl line',
        field_path: 'line',
        details: { message: (error as Error)?.message ?? String(error), line },
      });
      continue;
    }

    if (typeof sdk.validateRunnerCommand !== 'function') {
      rejected_count++;
      events.push({
        type: 'command_rejected',
        reason: 'command validator unavailable in sdk',
        field_path: 'command',
        details: { line },
      });
      continue;
    }

    const validated = sdk.validateRunnerCommand(payload);
    if (!validated.ok) {
      const command = payload && typeof payload === 'object' ? (payload as { id?: string; ts?: string; kind?: string }) : undefined;
      rejected_count++;
      events.push({
        type: 'command_rejected',
        command: command ? { id: command.id, ts: command.ts, kind: command.kind } : undefined,
        reason: validated.error.reason,
        field_path: validated.error.field_path,
        details: validated.error.details,
      });
      continue;
    }

    const command = validated.command;
    if (seenCommandIds.has(command.id)) {
      rejected_count++;
      events.push({
        type: 'command_rejected',
        command: { id: command.id, ts: command.ts, kind: command.kind },
        reason: 'duplicate command id',
        field_path: 'id',
      });
      continue;
    }

    const summary =
      typeof sdk.summarizeCommand === 'function'
        ? sdk.summarizeCommand(command)
        : { id: command.id, ts: command.ts, kind: command.kind };

    const execution = executeCommand({ sdk, plan, context, command, pausedNodeIds });
    if (!execution.ok) {
      rejected_count++;
      if (execution.audit_event) events.push(execution.audit_event);
      events.push({
        type: 'command_rejected',
        command: { id: command.id, ts: command.ts, kind: command.kind },
        reason: execution.reason,
        field_path: execution.field_path,
        details: execution.details,
      });
      continue;
    }

    seenCommandIds.add(command.id);
    accepted_count++;
    rerun_requested = rerun_requested || execution.rerun_requested;
    cancel_requested = cancel_requested || execution.cancel_requested;
    if (execution.events?.length) events.push(...execution.events);
    events.push({
      type: 'command_accepted',
      command: { id: command.id, ts: command.ts, kind: command.kind },
      details: {
        summary,
        applied: execution.applied,
        action: execution.action,
      },
    });
  }

  return {
    events,
    accepted_count,
    rejected_count,
    rerun_requested,
    cancel_requested,
  };
}

function executeCommand(args: {
  sdk: RunnerSdkModule;
  plan: RunnerPlan;
  context: RunnerContext;
  command: ParsedRunnerCommand;
  pausedNodeIds: Set<string>;
}):
  | {
      ok: true;
      applied: boolean;
      action: string;
      rerun_requested: boolean;
      cancel_requested: boolean;
      events?: RunnerEngineEvent[];
    }
  | {
      ok: false;
      reason: string;
      field_path?: string;
      details?: unknown;
      audit_event?: Extract<RunnerEngineEvent, { type: 'patch_rejected' }>;
    } {
  const { sdk, plan, context, command, pausedNodeIds } = args;
  ensureRuntimeCommandBuckets(context);

  if (command.kind === 'apply_patches') {
    const commandMeta = { id: command.id, ts: command.ts, kind: command.kind };
    const guardPolicy = getPatchGuardPolicy(context);
    try {
      const result = sdk.applyRuntimePatches(context, command.payload.patches, {
        guard: {
          enabled: true,
          policy: guardPolicy,
        },
      });

      const summary = result?.audit && isRecord(result.audit)
        ? {
            patch_count: asNumber(result.audit.patch_count) ?? command.payload.patches.length,
            applied_count: asNumber(result.audit.applied_count) ?? command.payload.patches.length,
            rejected_count: asNumber(result.audit.rejected_count) ?? 0,
            affected_paths: asStringArray(result.audit.affected_paths),
            partial_success: Boolean(result.audit.partial_success),
            hash: asString(result.audit.hash) ?? patchSummaryHash(sdk, command.payload.patches),
          }
        : {
            patch_count: command.payload.patches.length,
            applied_count: command.payload.patches.length,
            rejected_count: 0,
            affected_paths: command.payload.patches.map((p: { path: string }) => String(p.path)),
            partial_success: false,
            hash: patchSummaryHash(sdk, command.payload.patches),
          };

      return {
        ok: true,
        applied: true,
        action: 'apply_patches',
        rerun_requested: true,
        cancel_requested: false,
        events: [
          {
            type: 'patch_applied',
            command: commandMeta,
            summary,
            details: {
              policy: guardPolicy,
            },
          },
        ],
      };
    } catch (error) {
      const message = (error as Error)?.message ?? String(error);
      const errorDetails = isRecord((error as { details?: unknown })?.details)
        ? ((error as { details?: Record<string, unknown> }).details ?? {})
        : {};
      const rejectedPath = asString(errorDetails.path) ?? asString(asRecord(errorDetails.patch)?.path);
      const summary = {
        patch_count: command.payload.patches.length,
        applied_count: asNumber(errorDetails.applied_count) ?? 0,
        rejected_count: command.payload.patches.length,
        affected_paths: [] as string[],
        partial_success: false,
        hash: patchSummaryHash(sdk, command.payload.patches),
      };
      const audit_event: Extract<RunnerEngineEvent, { type: 'patch_rejected' }> = {
        type: 'patch_rejected',
        command: commandMeta,
        reason: message,
        field_path: rejectedPath ? 'payload.patches.path' : 'payload.patches',
        summary,
        details: {
          ...errorDetails,
          policy: guardPolicy,
        },
      };
      return {
        ok: false,
        reason: message,
        field_path: rejectedPath ? 'payload.patches.path' : 'payload.patches',
        details: {
          ...errorDetails,
          policy: guardPolicy,
          patch_summary: summary,
        },
        audit_event,
      };
    }
  }

  if (command.kind === 'user_confirm') {
    const nodeId = command.payload.node_id;
    if (!pausedNodeIds.has(nodeId)) {
      return {
        ok: false,
        reason: 'node_id is not in current paused set',
        field_path: 'payload.node_id',
        details: { node_id: nodeId, paused_node_ids: Array.from(pausedNodeIds) },
      };
    }
    const node = plan.nodes.find((entry) => entry.id === nodeId);
    if (!node) {
      return {
        ok: false,
        reason: 'node_id not found in execution plan',
        field_path: 'payload.node_id',
        details: { node_id: nodeId },
      };
    }

    const approved = command.payload.approve !== false;
    const nodeApprovals = asRecord(context.runtime.policy.runner_approvals) ?? {};
    nodeApprovals[nodeId] = {
      approved,
      ts: command.ts,
      action_ref: buildActionRef(node),
      node_id: nodeId,
    };
    context.runtime.policy.runner_approvals = nodeApprovals;

    return {
      ok: true,
      applied: true,
      action: approved ? 'user_confirm.approve' : 'user_confirm.reject',
      rerun_requested: approved,
      cancel_requested: !approved,
    };
  }

  if (command.kind === 'select_provider') {
    const overrides = asDetectOverrides(context.runtime.ctx.runner_detect_overrides);
    overrides.push({
      kind: command.payload.detect_kind,
      provider: command.payload.provider,
      chain: command.payload.chain,
      node_id: command.payload.node_id,
      ts: command.ts,
    });
    context.runtime.ctx.runner_detect_overrides = overrides;
    return {
      ok: true,
      applied: true,
      action: 'select_provider',
      rerun_requested: true,
      cancel_requested: false,
    };
  }

  const nodeId = command.payload.node_id;
  const cancelledByNode = asRecord(context.runtime.policy.runner_cancelled_by_node) ?? {};
  if (nodeId) cancelledByNode[nodeId] = true;
  context.runtime.policy.runner_cancelled_by_node = cancelledByNode;
  if (command.payload.reason) context.runtime.policy.runner_cancel_reason = command.payload.reason;
  return {
    ok: true,
    applied: true,
    action: 'cancel',
    rerun_requested: false,
    cancel_requested: true,
  };
}

function buildActionRef(node: RunnerPlan['nodes'][number]): string | undefined {
  const protocol = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const action = typeof node.source?.action === 'string' ? node.source.action : '';
  if (!protocol || !action) return undefined;
  return `${protocol}/${action}`;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function asDetectOverrides(value: unknown): Array<{
  kind: string;
  provider: string;
  chain?: string;
  node_id?: string;
  ts?: string;
}> {
  if (!Array.isArray(value)) return [];
  const out: Array<{ kind: string; provider: string; chain?: string; node_id?: string; ts?: string }> = [];
  for (const entry of value) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) continue;
    const rec = entry as Record<string, unknown>;
    if (typeof rec.kind !== 'string' || rec.kind.length === 0) continue;
    if (typeof rec.provider !== 'string' || rec.provider.length === 0) continue;
    out.push({
      kind: rec.kind,
      provider: rec.provider,
      chain: typeof rec.chain === 'string' ? rec.chain : undefined,
      node_id: typeof rec.node_id === 'string' ? rec.node_id : undefined,
      ts: typeof rec.ts === 'string' ? rec.ts : undefined,
    });
  }
  return out;
}

function ensureRuntimeCommandBuckets(context: RunnerContext): void {
  const runtime = asRecord((context as { runtime?: unknown }).runtime);
  if (!runtime) return;
  if (!asRecord(runtime.policy)) runtime.policy = {};
  if (!asRecord(runtime.ctx)) runtime.ctx = {};
}

function getPatchGuardPolicy(context: RunnerContext): {
  allow_roots: string[];
  allow_path_patterns?: string[];
  allow_nodes_paths?: string[];
} {
  const configured = asRecord(asRecord(context.runtime.policy)?.runner_patch_guard);
  return {
    allow_roots: asStringArray(configured?.allow_roots).length
      ? asStringArray(configured?.allow_roots)
      : ['inputs', 'ctx', 'contracts', 'policy'],
    allow_path_patterns: asStringArray(configured?.allow_path_patterns),
    allow_nodes_paths: asStringArray(configured?.allow_nodes_paths),
  };
}

function patchSummaryHash(sdk: RunnerSdkModule, patches: unknown): string {
  const serialized =
    typeof sdk.stringifyAisJson === 'function'
      ? sdk.stringifyAisJson(patches)
      : JSON.stringify(patches);
  return createHash('sha256').update(serialized).digest('hex');
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0);
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}
