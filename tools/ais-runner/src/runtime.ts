type WorkflowInputSpec = { type?: string; required?: boolean; default?: unknown };
type ParamSpec = { name: string; type: string; required?: boolean; default?: unknown };

export function ensureCtxNow(runtimeCtx: Record<string, unknown>): void {
  if (runtimeCtx.now !== undefined) return;
  runtimeCtx.now = BigInt(Math.floor(Date.now() / 1000));
}

export function coerceWorkflowInputs(
  declared: Record<string, WorkflowInputSpec> | undefined,
  provided: Record<string, unknown>
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...provided };
  if (!declared) return out;

  for (const [key, spec] of Object.entries(declared)) {
    if (out[key] === undefined && spec.default !== undefined) out[key] = spec.default;
    if (out[key] !== undefined) out[key] = coerceByType(spec.type, out[key]);
  }
  return out;
}

export function coerceArgsByParams(
  params: ParamSpec[] | undefined,
  provided: Record<string, unknown>
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...provided };
  if (!params) return out;
  const byName = new Map(params.map((p) => [p.name, p] as const));

  for (const [k, v] of Object.entries(out)) {
    const p = byName.get(k);
    if (!p) continue;
    out[k] = coerceByType(p.type, v);
  }

  // Apply defaults for missing keys.
  for (const p of params) {
    if (out[p.name] === undefined && p.default !== undefined) {
      out[p.name] = coerceByType(p.type, p.default);
    }
  }

  return out;
}

export function coerceByType(type: string | undefined, value: unknown): unknown {
  if (!type) return value;

  // Core integer-like types in AIS: treat as bigint when injected into runtime.
  if (isUintType(type) || isIntType(type)) return coerceBigInt(value, { allowNull: true });

  // Addresses and other strings: keep as string.
  if (type === 'address' || type === 'string' || type === 'hex') {
    if (value === null || value === undefined) return value;
    return String(value);
  }

  // Token amounts are frequently human strings; keep as string unless caller passed bigint already.
  if (type === 'token_amount') {
    if (value === null || value === undefined) return value;
    if (typeof value === 'bigint') return value;
    return String(value);
  }

  // Assets are structured objects; avoid implicit coercion here.
  if (type === 'asset') return value;

  return value;
}

function isUintType(t: string): boolean {
  return t === 'uint' || /^uint\d+$/.test(t);
}

function isIntType(t: string): boolean {
  return t === 'int' || /^int\d+$/.test(t);
}

function coerceBigInt(value: unknown, opts: { allowNull: boolean }): bigint | null | undefined {
  if (value === null) return opts.allowNull ? null : undefined;
  if (value === undefined) return undefined;
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || !Number.isInteger(value)) {
      throw new Error(`Expected integer for bigint coercion, got number=${String(value)}`);
    }
    return BigInt(value);
  }
  if (typeof value === 'string') {
    const s = value.trim();
    if (s === '') throw new Error('Expected integer string for bigint coercion, got empty string');
    if (!/^-?\d+$/.test(s)) throw new Error(`Expected integer string for bigint coercion, got "${value}"`);
    return BigInt(s);
  }
  throw new Error(`Expected bigint-coercible value, got ${typeof value}`);
}

