/**
 * RFC 8785-like JSON canonicalization (JCS).
 *
 * AIS uses canonical JSON as the recommended input for registry `specHash`.
 *
 * Notes:
 * - This implementation focuses on AIS document shapes (YAML-parsed JSON-like values).
 * - It rejects `undefined`, functions, symbols, and non-finite numbers.
 * - Object keys are sorted lexicographically.
 * - Arrays preserve order.
 */

import { keccak256 } from '../execution/evm/keccak.js';

export class JcsCanonicalizeError extends Error {
  constructor(message: string, public readonly details?: unknown) {
    super(message);
    this.name = 'JcsCanonicalizeError';
  }
}

export function canonicalizeJcs(value: unknown): string {
  return canonicalizeAny(value, '$');
}

/**
 * Convenience helper for AIS registry `specHash`.
 *
 * Returns `keccak256(utf8(canonicalizeJcs(spec)))`.
 */
export function specHashKeccak256(spec: unknown): string {
  return keccak256(canonicalizeJcs(spec));
}

function canonicalizeAny(value: unknown, path: string): string {
  if (value === null) return 'null';

  const t = typeof value;
  if (t === 'string') return JSON.stringify(value);
  if (t === 'boolean') return value ? 'true' : 'false';

  if (t === 'number') {
    if (!Number.isFinite(value)) {
      throw new JcsCanonicalizeError('Non-finite number is not allowed in canonical JSON', { path, value });
    }
    // JSON.stringify provides a deterministic ECMAScript number encoding which matches JCS requirements.
    return JSON.stringify(value);
  }

  if (t === 'bigint') {
    throw new JcsCanonicalizeError('bigint is not JSON-serializable; use string instead', { path });
  }

  if (t === 'undefined' || t === 'function' || t === 'symbol') {
    throw new JcsCanonicalizeError(`Unsupported JSON value type: ${t}`, { path });
  }

  if (value instanceof Uint8Array) {
    throw new JcsCanonicalizeError('Uint8Array is not JSON-serializable; use base64/hex string instead', { path });
  }

  if (Array.isArray(value)) {
    const parts = value.map((v, i) => canonicalizeAny(v, `${path}[${i}]`));
    return `[${parts.join(',')}]`;
  }

  if (t === 'object') {
    const obj = value as Record<string, unknown>;
    const keys = Object.keys(obj).sort();
    const parts: string[] = [];
    for (const k of keys) {
      const v = obj[k];
      if (v === undefined) {
        throw new JcsCanonicalizeError('Object contains undefined value (not allowed in JSON)', { path: `${path}.${k}` });
      }
      parts.push(`${JSON.stringify(k)}:${canonicalizeAny(v, `${path}.${k}`)}`);
    }
    return `{${parts.join(',')}}`;
  }

  throw new JcsCanonicalizeError(`Unsupported JSON value`, { path, type: t });
}

