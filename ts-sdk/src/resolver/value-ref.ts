/**
 * ValueRef evaluation (AIS 0.0.2)
 *
 * Resolves:
 * - {lit: ...}    → literal value
 * - {ref: "..."}  → runtime reference lookup
 * - {cel: "..."}  → CEL evaluation against runtime root
 * - {detect: ...} → provider-driven dynamic resolution
 * - {object: ...} / {array: ...} → recursive evaluation
 */

import type { Detect, ValueRef } from '../schema/index.js';
import { evaluateCEL, type CELContext, type CELValue } from '../cel/index.js';
import type { ResolverContext } from './context.js';
import { getRefFromRoot, getRuntimeRoot } from './context.js';

export class ValueRefEvalError extends Error {
  readonly refPath?: string;

  constructor(message: string, opts?: { refPath?: string; cause?: unknown }) {
    super(message);
    this.name = 'ValueRefEvalError';
    this.refPath = opts?.refPath;
    if (opts?.cause !== undefined) (this as { cause?: unknown }).cause = opts.cause;
  }
}

export interface DetectResolver {
  resolve(detect: Detect, ctx: ResolverContext): unknown | Promise<unknown>;
}

export interface EvaluateValueRefOptions {
  detect?: DetectResolver;
  /**
   * Engine capability IDs available to this evaluation. When omitted, the evaluator
   * will read `ctx.runtime.ctx.capabilities` if it is an array of strings.
   */
  capabilities?: string[];
  /**
   * Shallow overrides applied to the runtime root for this evaluation.
   * Common use: `{ params: <nodeParams> }` to isolate per-node parameters.
   */
  root_overrides?: Record<string, unknown>;
}

export function evaluateValueRef(
  value: ValueRef,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions = {}
): unknown {
  return evaluateValueRefSync(value, ctx, options);
}

export async function evaluateValueRefAsync(
  value: ValueRef,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions = {}
): Promise<unknown> {
  return await evaluateValueRefAsyncInternal(value, ctx, options);
}

function evaluateValueRefSync(
  value: ValueRef,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions
): unknown {
  const rootBase = getRuntimeRoot(ctx);
  const root =
    options.root_overrides ? { ...rootBase, ...options.root_overrides } : rootBase;
  if ('lit' in value) return value.lit;

  if ('ref' in value) {
    const resolved = getRefFromRoot(root, value.ref);
    if (resolved === undefined) {
      throw new ValueRefEvalError(`Missing ref: ${value.ref}`, { refPath: value.ref });
    }
    return resolved;
  }

  if ('cel' in value) {
    const celCtx: CELContext = toCELContext(root);
    try {
      return evaluateCEL(value.cel, celCtx);
    } catch (e) {
      throw new ValueRefEvalError(`CEL eval failed: ${value.cel}`, { cause: e });
    }
  }

  if ('detect' in value) {
    const out = resolveDetect(value.detect, ctx, options);
    if (out instanceof Promise) {
      throw new ValueRefEvalError('Async detect used in evaluateValueRef(); use evaluateValueRefAsync()');
    }
    if (isValueRefLike(out)) return evaluateValueRefSync(out, ctx, options);
    return out;
  }

  if ('object' in value) {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value.object)) {
      out[k] = evaluateValueRefSync(v, ctx, options);
    }
    return out;
  }

  if ('array' in value) {
    return value.array.map((v) => evaluateValueRefSync(v, ctx, options));
  }

  // Exhaustive check
  const _never: never = value;
  return _never;
}

async function evaluateValueRefAsyncInternal(
  value: ValueRef,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions
): Promise<unknown> {
  const rootBase = getRuntimeRoot(ctx);
  const root =
    options.root_overrides ? { ...rootBase, ...options.root_overrides } : rootBase;
  if ('lit' in value) return value.lit;

  if ('ref' in value) {
    const resolved = getRefFromRoot(root, value.ref);
    if (resolved === undefined) {
      throw new ValueRefEvalError(`Missing ref: ${value.ref}`, { refPath: value.ref });
    }
    return resolved;
  }

  if ('cel' in value) {
    const celCtx: CELContext = toCELContext(root);
    try {
      return evaluateCEL(value.cel, celCtx);
    } catch (e) {
      throw new ValueRefEvalError(`CEL eval failed: ${value.cel}`, { cause: e });
    }
  }

  if ('detect' in value) {
    const out = await resolveDetect(value.detect, ctx, options);
    if (isValueRefLike(out)) return await evaluateValueRefAsyncInternal(out, ctx, options);
    return out;
  }

  if ('object' in value) {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value.object)) {
      out[k] = await evaluateValueRefAsyncInternal(v, ctx, options);
    }
    return out;
  }

  if ('array' in value) {
    const out: unknown[] = [];
    for (const item of value.array) {
      out.push(await evaluateValueRefAsyncInternal(item, ctx, options));
    }
    return out;
  }

  const _never: never = value;
  return _never;
}

function resolveDetect(
  detect: Detect,
  ctx: ResolverContext,
  options: EvaluateValueRefOptions
): unknown | Promise<unknown> {
  const requiredCaps = detect.requires_capabilities ?? [];
  if (requiredCaps.length > 0) {
    const supported = getSupportedCapabilities(ctx, options);
    const missing = requiredCaps.filter((c) => !supported.has(c));
    if (missing.length > 0) {
      throw new ValueRefEvalError(`Detect requires missing capabilities: ${missing.join(', ')}`);
    }
  }

  // Built-in minimal behavior for choose_one when candidates are supplied.
  if (detect.kind === 'choose_one') {
    const candidates = detect.candidates ?? [];
    if (candidates.length === 0) {
      throw new ValueRefEvalError('detect.choose_one requires non-empty candidates');
    }
    return candidates[0];
  }

  if (!options.detect) {
    const providerInfo = detect.provider ? ` (provider: ${detect.provider})` : '';
    throw new ValueRefEvalError(`Detect kind "${detect.kind}" unsupported without resolver${providerInfo}`);
  }
  return options.detect.resolve(detect, ctx);
}

function getSupportedCapabilities(ctx: ResolverContext, options: EvaluateValueRefOptions): Set<string> {
  const fromOptions = options.capabilities;
  if (Array.isArray(fromOptions) && fromOptions.every((x) => typeof x === 'string')) {
    return new Set(fromOptions);
  }
  const fromCtx = (ctx.runtime.ctx as any)?.capabilities;
  if (Array.isArray(fromCtx) && fromCtx.every((x: unknown) => typeof x === 'string')) {
    return new Set(fromCtx as string[]);
  }
  return new Set();
}

function isValueRefLike(v: unknown): v is ValueRef {
  if (!v || typeof v !== 'object') return false;
  const keys = Object.keys(v as Record<string, unknown>);
  if (keys.length !== 1) return false;
  const k = keys[0];
  return k === 'lit' || k === 'ref' || k === 'cel' || k === 'detect' || k === 'object' || k === 'array';
}

function toCELContext(obj: Record<string, unknown>): CELContext {
  const out: Record<string, CELValue> = {};
  for (const [k, v] of Object.entries(obj)) {
    out[k] = toCELValue(v);
  }
  return out;
}

function toCELValue(v: unknown): CELValue {
  if (v === null) return null;
  if (typeof v === 'string' || typeof v === 'boolean') return v;
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number') {
    if (!Number.isFinite(v)) throw new ValueRefEvalError('CEL cannot accept non-finite number');
    if (!Number.isInteger(v)) {
      throw new ValueRefEvalError(
        'CEL disallows non-integer JS numbers; pass a decimal string instead'
      );
    }
    if (!Number.isSafeInteger(v)) {
      throw new ValueRefEvalError(
        'CEL disallows unsafe integer JS numbers; pass a bigint or integer string instead'
      );
    }
    return BigInt(v);
  }
  if (Array.isArray(v)) return v.map((x) => toCELValue(x));
  if (v && typeof v === 'object') {
    const out: Record<string, CELValue> = {};
    for (const [k, vv] of Object.entries(v as Record<string, unknown>)) {
      out[k] = toCELValue(vv);
    }
    return out;
  }
  return String(v);
}
