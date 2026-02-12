import type { ResolverContext } from '../resolver/index.js';
import { getRef, setRef } from '../resolver/index.js';
import { createHash } from 'node:crypto';
import { z } from 'zod';
import { stringifyAisJson } from './json.js';

export type RuntimePatchOp = 'set' | 'merge';

export interface RuntimePatch {
  op: RuntimePatchOp;
  path: string;
  value: unknown;
  extensions?: Record<string, unknown>;
}

export class RuntimePatchError extends Error {
  constructor(
    message: string,
    public readonly details?: unknown,
    public readonly code:
      | 'invalid_patch'
      | 'guard_rejected'
      | 'merge_type_error'
      | 'invalid_guard_config'
      | 'invalid_patches_input' = 'invalid_patch'
  ) {
    super(message);
    this.name = 'RuntimePatchError';
  }
}

export const RuntimePatchOpSchema = z.enum(['set', 'merge']);
export const RuntimePatchSchema = z
  .object({
    op: RuntimePatchOpSchema,
    path: z.string().min(1),
    value: z.unknown(),
    extensions: z.record(z.unknown()).optional(),
  })
  .strict();

export interface RuntimePatchValidationError {
  reason: string;
  field_path?: string;
  details?: unknown;
}

export function validateRuntimePatch(
  input: unknown
): { ok: true; patch: RuntimePatch } | { ok: false; error: RuntimePatchValidationError } {
  const parsed = RuntimePatchSchema.safeParse(input);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    return {
      ok: false,
      error: {
        reason: first?.message ?? 'invalid runtime patch',
        field_path: first?.path?.join('.') || undefined,
        details: parsed.error.issues.map((issue) => ({
          path: issue.path.join('.'),
          message: issue.message,
          code: issue.code,
        })),
      },
    };
  }
  return { ok: true, patch: parsed.data as RuntimePatch };
}

export interface RuntimePatchGuardPolicy {
  allow_roots: string[];
  allow_path_patterns?: string[];
  allow_nodes_paths?: string[];
}

export interface RuntimePatchGuardOptions {
  enabled?: boolean;
  policy?: Partial<RuntimePatchGuardPolicy>;
}

export interface RuntimePatchGuardRejection {
  index: number;
  path: string;
  reason: string;
  patch: RuntimePatch;
}

export interface RuntimePatchAuditEntry {
  index: number;
  op: RuntimePatchOp;
  path: string;
  status: 'applied' | 'rejected';
  reason?: string;
}

export interface RuntimePatchAuditSummary {
  patch_count: number;
  applied_count: number;
  rejected_count: number;
  affected_paths: string[];
  partial_success: boolean;
  hash: string;
}

export const DEFAULT_RUNTIME_PATCH_GUARD_POLICY: RuntimePatchGuardPolicy = {
  allow_roots: ['inputs', 'ctx', 'contracts', 'policy'],
  allow_path_patterns: [],
  allow_nodes_paths: [],
};

export function buildRuntimePatchGuardPolicy(
  policy?: Partial<RuntimePatchGuardPolicy>
): RuntimePatchGuardPolicy {
  return {
    allow_roots: normalizeStringArray(policy?.allow_roots, DEFAULT_RUNTIME_PATCH_GUARD_POLICY.allow_roots),
    allow_path_patterns: normalizeStringArray(policy?.allow_path_patterns, []),
    allow_nodes_paths: normalizeStringArray(policy?.allow_nodes_paths, []),
  };
}

export interface ApplyRuntimePatchOptions {
  /**
   * When true, captures undo patches that revert the applied changes.
   *
   * Note: undo is "best effort" for shallow merges (it stores the pre-merge value
   * at the path, not per-key diffs).
   */
  record_undo?: boolean;
  guard?: RuntimePatchGuardOptions;
  continue_on_error?: boolean;
}

export interface ApplyRuntimePatchResult {
  undo: RuntimePatch[];
  applied_count: number;
  rejected_count: number;
  affected_paths: string[];
  partial_success: boolean;
  rejected: RuntimePatchGuardRejection[];
  audit: RuntimePatchAuditSummary;
  audit_entries: RuntimePatchAuditEntry[];
}

export function applyRuntimePatch(
  ctx: ResolverContext,
  patch: RuntimePatch,
  options: ApplyRuntimePatchOptions = {}
): ApplyRuntimePatchResult {
  return applyRuntimePatches(ctx, [patch], options);
}

export function applyRuntimePatches(
  ctx: ResolverContext,
  patches: RuntimePatch[],
  options: ApplyRuntimePatchOptions = {}
): ApplyRuntimePatchResult {
  if (!Array.isArray(patches)) throw new RuntimePatchError('patches must be an array', undefined, 'invalid_patches_input');
  const guardEnabled = options.guard?.enabled === true;
  const guardPolicy = buildRuntimePatchGuardPolicy(options.guard?.policy);
  const continueOnError = options.continue_on_error === true;
  const undoAll: RuntimePatch[] = [];
  const auditEntries: RuntimePatchAuditEntry[] = [];
  const rejected: RuntimePatchGuardRejection[] = [];
  const affectedPaths = new Set<string>();
  let appliedCount = 0;

  for (let index = 0; index < patches.length; index++) {
    const rawPatch = patches[index];
    const validated = validateRuntimePatch(rawPatch);
    if (!validated.ok) {
      const reason = validated.error.reason;
      const details = { index, patch: rawPatch, ...validated.error };
      if (!continueOnError) throw new RuntimePatchError(reason, details, 'invalid_patch');
      auditEntries.push({ index, op: 'set', path: String((rawPatch as { path?: unknown })?.path ?? ''), status: 'rejected', reason });
      rejected.push({
        index,
        path: String((rawPatch as { path?: unknown })?.path ?? ''),
        reason,
        patch: { op: 'set', path: String((rawPatch as { path?: unknown })?.path ?? ''), value: undefined },
      });
      continue;
    }

    const patch = validated.patch;
    if (guardEnabled) {
      const guardCheck = checkRuntimePatchPathAllowed(patch.path, guardPolicy);
      if (!guardCheck.ok) {
        const rejection: RuntimePatchGuardRejection = { index, path: patch.path, reason: guardCheck.reason, patch };
        const details = {
          ...rejection,
          policy: guardPolicy,
        };
        if (!continueOnError) throw new RuntimePatchError('Runtime patch rejected by guard', details, 'guard_rejected');
        rejected.push(rejection);
        auditEntries.push({ index, op: patch.op, path: patch.path, status: 'rejected', reason: guardCheck.reason });
        continue;
      }
    }

    try {
      const prev = getRef(ctx, patch.path);
      if (options.record_undo) undoAll.push({ op: 'set', path: patch.path, value: prev });
      if (patch.op === 'set') {
        setRef(ctx, patch.path, patch.value);
      } else {
        if (patch.value === null || typeof patch.value !== 'object' || Array.isArray(patch.value)) {
          throw new RuntimePatchError('merge patch.value must be a plain object', { index, path: patch.path }, 'merge_type_error');
        }
        if (prev === undefined) {
          setRef(ctx, patch.path, patch.value);
        } else if (prev === null || typeof prev !== 'object' || Array.isArray(prev)) {
          throw new RuntimePatchError(
            'Cannot merge into non-object existing value',
            { index, path: patch.path, prevType: typeof prev },
            'merge_type_error'
          );
        } else {
          setRef(ctx, patch.path, { ...(prev as Record<string, unknown>), ...(patch.value as Record<string, unknown>) });
        }
      }
      appliedCount++;
      affectedPaths.add(patch.path);
      auditEntries.push({ index, op: patch.op, path: patch.path, status: 'applied' });
    } catch (error) {
      if (!continueOnError) throw error;
      const reason = (error as Error)?.message ?? String(error);
      rejected.push({ index, path: patch.path, reason, patch });
      auditEntries.push({ index, op: patch.op, path: patch.path, status: 'rejected', reason });
    }
  }

  const patchCount = patches.length;
  const rejectedCount = patchCount - appliedCount;
  const partialSuccess = appliedCount > 0 && rejectedCount > 0;
  const affected = Array.from(affectedPaths);
  const hash = createHash('sha256').update(stringifyAisJson(auditEntries)).digest('hex');

  return {
    undo: undoAll,
    applied_count: appliedCount,
    rejected_count: rejectedCount,
    affected_paths: affected,
    partial_success: partialSuccess,
    rejected,
    audit_entries: auditEntries,
    audit: {
      patch_count: patchCount,
      applied_count: appliedCount,
      rejected_count: rejectedCount,
      affected_paths: affected,
      partial_success: partialSuccess,
      hash,
    },
  };
}

export function checkRuntimePatchPathAllowed(
  path: string,
  policy: RuntimePatchGuardPolicy = DEFAULT_RUNTIME_PATCH_GUARD_POLICY
): { ok: true } | { ok: false; reason: string } {
  const normalized = String(path ?? '').trim();
  if (!normalized) return { ok: false, reason: 'empty patch path' };

  const root = normalized.split('.', 1)[0] ?? '';
  if (!root) return { ok: false, reason: 'empty patch path root' };
  if (root === 'nodes') {
    const allowedNodePatterns = normalizeStringArray(policy.allow_nodes_paths, []);
    if (allowedNodePatterns.length === 0) {
      return { ok: false, reason: 'patch path root "nodes" is blocked by guard policy' };
    }
    for (const pattern of allowedNodePatterns) {
      if (safeRegexTest(pattern, normalized)) return { ok: true };
    }
    return { ok: false, reason: 'nodes.* path is not matched by allow_nodes_paths guard patterns' };
  }

  const roots = normalizeStringArray(policy.allow_roots, DEFAULT_RUNTIME_PATCH_GUARD_POLICY.allow_roots);
  if (roots.includes(root)) return { ok: true };

  for (const pattern of normalizeStringArray(policy.allow_path_patterns, [])) {
    if (safeRegexTest(pattern, normalized)) return { ok: true };
  }

  return {
    ok: false,
    reason: `patch path root "${root}" is not allowed (allowed roots: ${roots.join(', ')})`,
  };
}

function normalizeStringArray(value: unknown, fallback: string[]): string[] {
  if (!Array.isArray(value)) return [...fallback];
  const out = value.filter((entry): entry is string => typeof entry === 'string' && entry.trim().length > 0).map((entry) => entry.trim());
  if (out.length === 0 && fallback.length > 0) return [...fallback];
  return out;
}

function safeRegexTest(pattern: string, value: string): boolean {
  try {
    return new RegExp(pattern).test(value);
  } catch {
    return false;
  }
}
