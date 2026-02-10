import type { ResolverContext } from '../resolver/index.js';
import { getRef, setRef } from '../resolver/index.js';

export type RuntimePatchOp = 'set' | 'merge';

export interface RuntimePatch {
  op: RuntimePatchOp;
  path: string;
  value: unknown;
}

export class RuntimePatchError extends Error {
  constructor(message: string, public readonly details?: unknown) {
    super(message);
    this.name = 'RuntimePatchError';
  }
}

export interface ApplyRuntimePatchOptions {
  /**
   * When true, captures undo patches that revert the applied changes.
   *
   * Note: undo is "best effort" for shallow merges (it stores the pre-merge value
   * at the path, not per-key diffs).
   */
  record_undo?: boolean;
}

export interface ApplyRuntimePatchResult {
  undo: RuntimePatch[];
}

export function applyRuntimePatch(
  ctx: ResolverContext,
  patch: RuntimePatch,
  options: ApplyRuntimePatchOptions = {}
): ApplyRuntimePatchResult {
  if (!patch || typeof patch !== 'object') {
    throw new RuntimePatchError('Invalid patch: must be an object');
  }
  if (patch.op !== 'set' && patch.op !== 'merge') {
    throw new RuntimePatchError(`Invalid patch.op: ${String((patch as any).op)}`);
  }
  if (typeof patch.path !== 'string' || patch.path.length === 0) {
    throw new RuntimePatchError('Invalid patch.path: must be a non-empty string');
  }

  const prev = getRef(ctx, patch.path);
  const undo: RuntimePatch[] = [];
  if (options.record_undo) {
    undo.push({ op: 'set', path: patch.path, value: prev });
  }

  if (patch.op === 'set') {
    setRef(ctx, patch.path, patch.value);
    return { undo };
  }

  // merge
  if (patch.value === null || typeof patch.value !== 'object' || Array.isArray(patch.value)) {
    throw new RuntimePatchError('merge patch.value must be a plain object');
  }

  if (prev === undefined) {
    setRef(ctx, patch.path, patch.value);
    return { undo };
  }
  if (prev === null || typeof prev !== 'object' || Array.isArray(prev)) {
    throw new RuntimePatchError('Cannot merge into non-object existing value', {
      path: patch.path,
      prevType: typeof prev,
    });
  }

  setRef(ctx, patch.path, { ...(prev as Record<string, unknown>), ...(patch.value as Record<string, unknown>) });
  return { undo };
}

export function applyRuntimePatches(
  ctx: ResolverContext,
  patches: RuntimePatch[],
  options: ApplyRuntimePatchOptions = {}
): ApplyRuntimePatchResult {
  if (!Array.isArray(patches)) throw new RuntimePatchError('patches must be an array');

  const undoAll: RuntimePatch[] = [];
  for (const p of patches) {
    const r = applyRuntimePatch(ctx, p, options);
    if (options.record_undo) undoAll.push(...r.undo);
  }
  return { undo: undoAll };
}

