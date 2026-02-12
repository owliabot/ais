import { randomUUID } from 'node:crypto';
import type { ExecutionPlan, ExecutionPlanNode } from '../execution/index.js';
import { applyRuntimePatches, type RuntimePatch, DEFAULT_RUNTIME_PATCH_GUARD_POLICY } from '../engine/patch.js';
import type { EngineEvent, RunPlanOptions } from '../engine/types.js';
import { runPlan } from '../engine/runner.js';

export type DeterministicAgentCommand =
  | {
      kind: 'apply_patches';
      id?: string;
      ts?: string;
      patches: RuntimePatch[];
    }
  | {
      kind: 'user_confirm';
      id?: string;
      ts?: string;
      node_id: string;
      approve?: boolean;
    }
  | { kind: 'cancel'; id?: string; ts?: string; node_id?: string; reason?: string };

export type DeterministicAgentConfig = {
  /**
   * Provide concrete values for missing refs. Keys are runtime ref paths like
   * `inputs.amount` or `ctx.wallet_address`.
   */
  fill?: Record<string, unknown>;
  /**
   * Maximum loop rounds (each round is a runPlan invocation until pause or completion).
   */
  max_rounds?: number;
  /**
   * When true, auto-approves need_user_confirm prompts via user_confirm.
   */
  auto_approve?: boolean;
  /**
   * Apply patches with guard enabled using this policy (defaults to SDK defaults).
   */
  patch_guard_policy?: {
    allow_roots?: string[];
    allow_path_patterns?: string[];
    allow_nodes_paths?: string[];
  };
  /**
   * Deterministic clock for commands. Defaults to `new Date().toISOString()`.
   */
  now?: () => string;
};

export type DeterministicAgentResult =
  | {
      ok: true;
      rounds: number;
      events: EngineEvent[];
      applied_patches: number;
      approved_nodes: string[];
    }
  | {
      ok: false;
      rounds: number;
      events: EngineEvent[];
      reason: string;
      details?: unknown;
    };

/**
 * AGT107: Minimal deterministic agent loop reference.
 *
 * This is a test harness style loop: it runs the engine until it pauses, then
 * applies preset patches and/or approvals, then resumes.
 */
export async function runDeterministicAgentLoop(args: {
  plan: ExecutionPlan;
  ctx: any; // ResolverContext (kept as any to avoid pulling resolver types into agent module)
  engine: RunPlanOptions;
  config?: DeterministicAgentConfig;
}): Promise<DeterministicAgentResult> {
  const { plan, ctx, engine } = args;
  const cfg = args.config ?? {};
  const now = cfg.now ?? (() => new Date().toISOString());
  const maxRounds = cfg.max_rounds ?? 16;
  const autoApprove = cfg.auto_approve ?? true;
  const fill = cfg.fill ?? {};

  const events: EngineEvent[] = [];
  let appliedPatches = 0;
  const approvedNodes = new Set<string>();

  for (let round = 0; round < maxRounds; round++) {
    let paused: Extract<EngineEvent, { type: 'engine_paused' }> | null = null;
    let errored: Extract<EngineEvent, { type: 'error' }> | null = null;

    for await (const ev of runPlan(plan, ctx, engine)) {
      events.push(ev);
      if (ev.type === 'engine_paused') {
        paused = ev;
        break;
      }
      if (ev.type === 'error') {
        errored = ev;
        break;
      }
    }

    if (errored) {
      return { ok: false, rounds: round + 1, events, reason: errored.error?.message ?? 'engine error', details: errored };
    }
    if (!paused) {
      return {
        ok: true,
        rounds: round + 1,
        events,
        applied_patches: appliedPatches,
        approved_nodes: Array.from(approvedNodes),
      };
    }

    const commands = buildCommandsForPaused({
      paused,
      fill,
      auto_approve: autoApprove,
      now,
    });
    if (commands.length === 0) {
      return { ok: false, rounds: round + 1, events, reason: 'agent cannot resolve paused state', details: paused };
    }

    const pausedNodeIds = new Set(paused.paused.map((p) => p.node.id));
    for (const cmd of commands) {
      const applied = applyCommand({
        plan,
        ctx,
        pausedNodeIds,
        cmd,
        now,
        guard_policy: cfg.patch_guard_policy,
      });
      if (!applied.ok) return { ok: false, rounds: round + 1, events, reason: applied.reason, details: applied.details };
      appliedPatches += applied.applied_patches;
      if (applied.approved_node_id) approvedNodes.add(applied.approved_node_id);
    }
  }

  return { ok: false, rounds: maxRounds, events, reason: 'max_rounds exceeded' };
}

function buildCommandsForPaused(args: {
  paused: Extract<EngineEvent, { type: 'engine_paused' }>;
  fill: Record<string, unknown>;
  auto_approve: boolean;
  now: () => string;
}): DeterministicAgentCommand[] {
  const { paused, fill, auto_approve, now } = args;
  const patches: RuntimePatch[] = [];
  const confirms: DeterministicAgentCommand[] = [];

  for (const entry of paused.paused) {
    const missing = collectMissingRefs(entry.details);
    for (const ref of missing) {
      if (!(ref in fill)) continue;
      patches.push({ op: 'set', path: ref, value: fill[ref] });
    }

    if (auto_approve) {
      // Approve any paused node that isn't unblocked by patches alone.
      confirms.push({ kind: 'user_confirm', node_id: entry.node.id, approve: true, ts: now() });
    }
  }

  const out: DeterministicAgentCommand[] = [];
  if (patches.length > 0) out.push({ kind: 'apply_patches', patches, ts: now() });
  out.push(...confirms);
  return out;
}

function applyCommand(args: {
  plan: ExecutionPlan;
  ctx: any;
  pausedNodeIds: Set<string>;
  cmd: DeterministicAgentCommand;
  now: () => string;
  guard_policy?: DeterministicAgentConfig['patch_guard_policy'];
}): { ok: true; applied_patches: number; approved_node_id?: string } | { ok: false; reason: string; details?: unknown } {
  const { ctx, pausedNodeIds, cmd } = args;
  if (cmd.kind === 'apply_patches') {
    const policy = {
      ...DEFAULT_RUNTIME_PATCH_GUARD_POLICY,
      ...(args.guard_policy ?? {}),
    };
    const res = applyRuntimePatches(ctx, cmd.patches, { guard: { enabled: true, policy } });
    return { ok: true, applied_patches: res.applied_count };
  }

  if (cmd.kind === 'user_confirm') {
    const nodeId = cmd.node_id;
    if (!pausedNodeIds.has(nodeId)) {
      return { ok: false, reason: 'node_id is not paused', details: { node_id: nodeId, paused: Array.from(pausedNodeIds) } };
    }
    ensureRuntimePolicyBuckets(ctx);
    const approvals = asRecord(ctx.runtime.policy.runner_approvals) ?? {};
    approvals[nodeId] = {
      approved: cmd.approve !== false,
      ts: cmd.ts ?? args.now(),
    };
    ctx.runtime.policy.runner_approvals = approvals;
    return { ok: true, applied_patches: 0, approved_node_id: nodeId };
  }

  // cancel
  ensureRuntimePolicyBuckets(ctx);
  const cancelled = asRecord(ctx.runtime.policy.runner_cancelled_by_node) ?? {};
  if (cmd.node_id) cancelled[cmd.node_id] = true;
  ctx.runtime.policy.runner_cancelled_by_node = cancelled;
  if (cmd.reason) ctx.runtime.policy.runner_cancel_reason = cmd.reason;
  return { ok: true, applied_patches: 0 };
}

function collectMissingRefs(details: unknown): string[] {
  const rec = asRecord(details);
  if (!rec) return [];
  const direct = asStringArray(rec.missing_refs);
  if (direct.length > 0) return direct;
  const readiness = asRecord(rec.readiness);
  if (!readiness) return [];
  return asStringArray(readiness.missing_refs);
}

function ensureRuntimePolicyBuckets(ctx: any): void {
  if (!ctx || typeof ctx !== 'object') return;
  if (!ctx.runtime || typeof ctx.runtime !== 'object') ctx.runtime = {};
  if (!ctx.runtime.policy || typeof ctx.runtime.policy !== 'object') ctx.runtime.policy = {};
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0);
}

