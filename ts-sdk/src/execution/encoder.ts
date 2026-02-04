/**
 * ABI Encoder - encode function calls for EVM transactions
 * Lightweight implementation without external dependencies
 */

import { keccak256 } from './keccak.js';

export type SolidityType =
  | 'address'
  | 'bool'
  | 'bytes'
  | 'string'
  | `bytes${number}`
  | `uint${number}`
  | `int${number}`
  | `${string}[]`
  | `${string}[${number}]`;

/**
 * Encode a function selector from function signature
 */
export function encodeFunctionSelector(signature: string): string {
  const hash = keccak256(signature);
  return hash.slice(0, 10); // 0x + 4 bytes
}

/**
 * Encode a single value for ABI encoding
 */
export function encodeValue(type: string, value: unknown): string {
  // Handle arrays
  if (type.endsWith('[]')) {
    const baseType = type.slice(0, -2);
    const arr = value as unknown[];
    // Dynamic array: offset + length + elements
    const encodedElements = arr.map((v) => encodeValue(baseType, v));
    const length = padLeft(arr.length.toString(16), 64);
    return length + encodedElements.join('');
  }

  // Handle fixed arrays
  const fixedMatch = type.match(/^(.+)\[(\d+)\]$/);
  if (fixedMatch) {
    const [, baseType, size] = fixedMatch;
    const arr = value as unknown[];
    if (arr.length !== parseInt(size)) {
      throw new Error(`Expected array of length ${size}, got ${arr.length}`);
    }
    return arr.map((v) => encodeValue(baseType, v)).join('');
  }

  // Handle basic types
  if (type === 'address') {
    return encodeAddress(value as string);
  }

  if (type === 'bool') {
    return padLeft(value ? '1' : '0', 64);
  }

  if (type === 'string') {
    return encodeString(value as string);
  }

  if (type === 'bytes') {
    return encodeBytes(value as string);
  }

  if (type.startsWith('bytes')) {
    const size = parseInt(type.slice(5));
    return encodeFixedBytes(value as string, size);
  }

  if (type.startsWith('uint') || type.startsWith('int')) {
    return encodeInteger(value, type);
  }

  // Tuple/struct support would go here
  throw new Error(`Unsupported type: ${type}`);
}

/**
 * Encode an address (20 bytes, left-padded to 32)
 */
function encodeAddress(value: string): string {
  const addr = value.toLowerCase().replace('0x', '');
  if (addr.length !== 40) {
    throw new Error(`Invalid address: ${value}`);
  }
  return padLeft(addr, 64);
}

/**
 * Encode an integer (uint/int)
 */
function encodeInteger(value: unknown, type: string): string {
  const isSigned = type.startsWith('int');
  let num: bigint;

  if (typeof value === 'bigint') {
    num = value;
  } else if (typeof value === 'number') {
    num = BigInt(value);
  } else if (typeof value === 'string') {
    // Handle hex strings
    if (value.startsWith('0x')) {
      num = BigInt(value);
    } else {
      num = BigInt(value);
    }
  } else {
    throw new Error(`Cannot encode ${typeof value} as ${type}`);
  }

  if (isSigned && num < 0n) {
    // Two's complement for negative numbers
    num = (1n << 256n) + num;
  }

  const hex = num.toString(16);
  return padLeft(hex, 64);
}

/**
 * Encode a dynamic string
 */
function encodeString(value: string): string {
  const bytes = new TextEncoder().encode(value);
  const hex = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
  const length = padLeft(bytes.length.toString(16), 64);
  const paddedData = hex.padEnd(Math.ceil(hex.length / 64) * 64, '0');
  return length + paddedData;
}

/**
 * Encode dynamic bytes
 */
function encodeBytes(value: string): string {
  const hex = value.replace('0x', '');
  const byteLength = hex.length / 2;
  const length = padLeft(byteLength.toString(16), 64);
  const paddedData = hex.padEnd(Math.ceil(hex.length / 64) * 64, '0');
  return length + paddedData;
}

/**
 * Encode fixed bytes (bytes1 to bytes32)
 */
function encodeFixedBytes(value: string, size: number): string {
  const hex = value.replace('0x', '');
  if (hex.length !== size * 2) {
    throw new Error(`Expected ${size} bytes, got ${hex.length / 2}`);
  }
  return hex.padEnd(64, '0');
}

/**
 * Left-pad a hex string to specified length
 */
function padLeft(hex: string, length: number): string {
  return hex.padStart(length, '0');
}

/**
 * Check if a type is dynamic (variable length)
 */
export function isDynamicType(type: string): boolean {
  if (type === 'string' || type === 'bytes') return true;
  if (type.endsWith('[]')) return true;
  // Fixed arrays of dynamic types are also dynamic
  const fixedMatch = type.match(/^(.+)\[\d+\]$/);
  if (fixedMatch && isDynamicType(fixedMatch[1])) return true;
  return false;
}

/**
 * Encode function call with parameters
 */
export function encodeFunctionCall(
  signature: string,
  types: string[],
  values: unknown[]
): string {
  if (types.length !== values.length) {
    throw new Error(`Parameter count mismatch: ${types.length} types, ${values.length} values`);
  }

  const selector = encodeFunctionSelector(signature);

  if (types.length === 0) {
    return selector;
  }

  // Separate static and dynamic parts
  const heads: string[] = [];
  const tails: string[] = [];
  let tailOffset = types.length * 32; // Initial offset after all heads

  for (let i = 0; i < types.length; i++) {
    const type = types[i];
    const value = values[i];

    if (isDynamicType(type)) {
      // Head contains offset to tail
      heads.push(padLeft(tailOffset.toString(16), 64));
      const encoded = encodeValue(type, value);
      tails.push(encoded);
      tailOffset += encoded.length / 2; // bytes, not hex chars
    } else {
      // Head contains the actual value
      heads.push(encodeValue(type, value));
    }
  }

  return selector + heads.join('') + tails.join('');
}

/**
 * Build function signature from name and param types
 */
export function buildFunctionSignature(name: string, types: string[]): string {
  return `${name}(${types.join(',')})`;
}
