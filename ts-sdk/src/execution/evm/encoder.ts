/**
 * ABI helpers for EVM execution (AIS 0.0.2)
 *
 * - Uses `ethers` for ABI encoding/decoding and keccak selectors.
 * - Adds AIS-specific strict argument alignment by JSON ABI input names.
 */

import { Interface, id } from 'ethers';
import type { JsonAbiFunction, JsonAbiParam } from '../../schema/index.js';

export function encodeFunctionSelector(signature: string): string {
  return id(signature).slice(0, 10); // 0x + 4 bytes
}

// ──────────────────────────────────────────────────────────────────────────────
// JSON ABI encoding/decoding (AIS 0.0.2)
// ──────────────────────────────────────────────────────────────────────────────

export class AbiArgsError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'AbiArgsError';
  }
}

export class AbiEncodingError extends Error {
  readonly path: string;

  constructor(message: string, path: string) {
    super(`${message} (at ${path})`);
    this.name = 'AbiEncodingError';
    this.path = path;
  }
}

export class AbiDecodingError extends Error {
  readonly path: string;

  constructor(message: string, path: string) {
    super(`${message} (at ${path})`);
    this.name = 'AbiDecodingError';
    this.path = path;
  }
}

type AbiType =
  | { kind: 'base'; type: string }
  | { kind: 'tuple'; components: JsonAbiParam[] }
  | { kind: 'array'; item: AbiType; length?: number };

export function buildFunctionSignatureFromJsonAbi(abi: JsonAbiFunction): string {
  const types = abi.inputs.map((p) => canonicalParamType(p));
  return `${abi.name}(${types.join(',')})`;
}

export function encodeJsonAbiFunctionCall(abi: JsonAbiFunction, args: Record<string, unknown>): string {
  validateJsonAbiParams(abi.inputs, 'abi.inputs', (m, p) => new AbiEncodingError(m, p));

  const values = alignArgsByName(abi, args);
  try {
    const iface = new Interface([abi as any]);
    return iface.encodeFunctionData(abi.name, values as any);
  } catch (e) {
    throw new AbiEncodingError(`ABI encoding failed: ${(e as Error)?.message ?? String(e)}`, 'args');
  }
}

/**
 * Decode EVM return data according to JSON ABI outputs.
 *
 * Returns an object keyed by `outputs[*].name`. Tuple outputs are decoded to an
 * object when all component names are present and unique; otherwise, an array.
 */
export function decodeJsonAbiFunctionResult(abi: JsonAbiFunction, returnData: string): Record<string, unknown> {
  const outputs = abi.outputs ?? [];
  if (outputs.length === 0) return {};

  validateJsonAbiParams(outputs, 'abi.outputs', (m, p) => new AbiDecodingError(m, p));

  const outNames = outputs.map((o) => o.name);
  const empty = outNames.filter((n) => n.length === 0);
  if (empty.length > 0) {
    throw new AbiDecodingError('JSON ABI outputs must have non-empty names', 'abi.outputs');
  }
  const seen = new Set<string>();
  for (const n of outNames) {
    if (seen.has(n)) throw new AbiDecodingError(`Duplicate JSON ABI output name: ${n}`, 'abi.outputs');
    seen.add(n);
  }

  let decoded: any;
  try {
    const iface = new Interface([abi as any]);
    decoded = iface.decodeFunctionResult(abi.name, returnData);
  } catch (e) {
    throw new AbiDecodingError(`ABI decoding failed: ${(e as Error)?.message ?? String(e)}`, 'returnData');
  }

  const types = outputs.map((p) => toAbiType(p, `abi.outputs.${p.name || '(unnamed)'}`));
  const out: Record<string, unknown> = {};
  for (let i = 0; i < outputs.length; i++) {
    const name = outputs[i]!.name;
    out[name] = convertDecoded(types[i]!, decoded[i], `outputs.${name}`);
  }
  return out;
}

function alignArgsByName(abi: JsonAbiFunction, args: Record<string, unknown>): unknown[] {
  const inputNames = abi.inputs.map((i) => i.name);
  const empty = inputNames.filter((n) => n.length === 0);
  if (empty.length > 0) {
    throw new AbiArgsError('JSON ABI inputs must have non-empty names for AIS args mapping');
  }
  const seen = new Set<string>();
  for (const n of inputNames) {
    if (seen.has(n)) throw new AbiArgsError(`Duplicate JSON ABI input name: ${n}`);
    seen.add(n);
  }

  const missing = inputNames.filter((n) => !(n in args));
  const extra = Object.keys(args).filter((k) => !seen.has(k));
  if (missing.length > 0 || extra.length > 0) {
    const parts: string[] = [];
    if (missing.length > 0) parts.push(`missing: ${missing.join(', ')}`);
    if (extra.length > 0) parts.push(`extra: ${extra.join(', ')}`);
    throw new AbiArgsError(`ABI args mismatch (${parts.join(' ; ')})`);
  }

  return inputNames.map((n) => args[n]);
}

function validateJsonAbiParams(
  params: JsonAbiParam[],
  rootPath: string,
  makeError: (message: string, path: string) => Error
): void {
  for (const p of params) {
    if (p.type.startsWith('tuple')) {
      if (!p.components || p.components.length === 0) {
        throw makeError('tuple type missing components', `${rootPath}.${p.name || '(unnamed)'}`);
      }
      validateJsonAbiParams(p.components, `${rootPath}.${p.name || '(unnamed)'}.components`, makeError);
    }
  }
}

function canonicalParamType(param: JsonAbiParam): string {
  const { base, suffixParts } = splitArraySuffixParts(param.type);
  const suffix = suffixParts.map((s) => `[${s.length ?? ''}]`).join('');

  if (base === 'tuple') {
    const comps = param.components ?? [];
    const inner = comps.map((c) => canonicalParamType(c)).join(',');
    return `(${inner})${suffix}`;
  }

  return `${base}${suffix}`;
}

function toAbiType(param: JsonAbiParam, path: string): AbiType {
  const { base, suffixParts } = splitArraySuffixParts(param.type);

  let inner: AbiType;
  if (base === 'tuple') {
    const components = param.components;
    if (!components || components.length === 0) {
      throw new AbiEncodingError('tuple type missing components', path);
    }
    inner = { kind: 'tuple', components };
  } else {
    inner = { kind: 'base', type: base };
  }

  for (const s of suffixParts) {
    inner = { kind: 'array', item: inner, length: s.length };
  }
  return inner;
}

function splitArraySuffixParts(type: string): { base: string; suffixParts: Array<{ length?: number }> } {
  const parts: Array<{ length?: number }> = [];
  let base = type;
  while (true) {
    const m = base.match(/^(.*)\[(\d*)\]$/);
    if (!m) break;
    base = m[1]!;
    const lenStr = m[2]!;
    parts.unshift(lenStr === '' ? {} : { length: Number(lenStr) });
  }
  return { base, suffixParts: parts };
}

function convertDecoded(type: AbiType, value: unknown, path: string): unknown {
  if (type.kind === 'array') {
    if (!Array.isArray(value)) {
      throw new AbiDecodingError(`Expected array value, got ${typeof value}`, path);
    }
    if (type.length !== undefined && value.length !== type.length) {
      throw new AbiDecodingError(`Expected array length ${type.length}, got ${value.length}`, path);
    }
    return value.map((v, i) => convertDecoded(type.item, v, `${path}[${i}]`));
  }

  if (type.kind === 'tuple') {
    if (!Array.isArray(value)) {
      throw new AbiDecodingError(`Expected tuple/array value, got ${typeof value}`, path);
    }
    const items = type.components.map((c, i) => {
      const childType = toAbiType(c, `${path}.${c.name || i}`);
      return convertDecoded(childType, (value as any)[i], `${path}.${c.name || i}`);
    });

    const names = type.components.map((c) => c.name);
    const allNamed = names.every((n) => n.length > 0);
    const unique = new Set(names).size === names.length;
    if (!allNamed || !unique) return items;

    const out: Record<string, unknown> = {};
    for (let i = 0; i < names.length; i++) out[names[i]!] = items[i];
    return out;
  }

  // base
  if (type.type === 'address') {
    if (typeof value !== 'string') {
      throw new AbiDecodingError(`Expected address string, got ${typeof value}`, path);
    }
    return value.toLowerCase();
  }

  return value;
}
