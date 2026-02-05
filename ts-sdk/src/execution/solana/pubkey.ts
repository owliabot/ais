/**
 * Solana PublicKey utilities
 */

import { base58Decode, base58Encode, isValidPublicKey } from './base58.js';
import type { PublicKey } from './types.js';

/**
 * Create a PublicKey from a Base58 string
 */
export function publicKeyFromBase58(base58: string): PublicKey {
  if (!isValidPublicKey(base58)) {
    throw new Error(`Invalid public key: ${base58}`);
  }
  const bytes = base58Decode(base58);
  return { bytes, base58 };
}

/**
 * Create a PublicKey from bytes
 */
export function publicKeyFromBytes(bytes: Uint8Array): PublicKey {
  if (bytes.length !== 32) {
    throw new Error(`Invalid public key length: ${bytes.length} (expected 32)`);
  }
  const base58 = base58Encode(bytes);
  return { bytes: new Uint8Array(bytes), base58 };
}

/**
 * Create a PublicKey from a hex string (with or without 0x prefix)
 */
export function publicKeyFromHex(hex: string): PublicKey {
  const cleanHex = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (cleanHex.length !== 64) {
    throw new Error(`Invalid hex public key length: ${cleanHex.length} (expected 64)`);
  }
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(cleanHex.slice(i * 2, i * 2 + 2), 16);
  }
  return publicKeyFromBytes(bytes);
}

/**
 * Compare two PublicKeys for equality
 */
export function publicKeysEqual(a: PublicKey, b: PublicKey): boolean {
  if (a.bytes.length !== b.bytes.length) return false;
  for (let i = 0; i < a.bytes.length; i++) {
    if (a.bytes[i] !== b.bytes[i]) return false;
  }
  return true;
}

/**
 * Convert PublicKey to hex string (no 0x prefix)
 */
export function publicKeyToHex(pubkey: PublicKey): string {
  return Array.from(pubkey.bytes)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');
}

/**
 * Check if a value can be converted to a PublicKey
 */
export function isPublicKeyLike(value: unknown): value is string | Uint8Array | PublicKey {
  if (typeof value === 'string') {
    return isValidPublicKey(value);
  }
  if (value instanceof Uint8Array) {
    return value.length === 32;
  }
  if (typeof value === 'object' && value !== null) {
    const pk = value as PublicKey;
    return pk.bytes instanceof Uint8Array && pk.bytes.length === 32;
  }
  return false;
}

/**
 * Convert any PublicKey-like value to PublicKey
 */
export function toPublicKey(value: string | Uint8Array | PublicKey): PublicKey {
  if (typeof value === 'string') {
    return publicKeyFromBase58(value);
  }
  if (value instanceof Uint8Array) {
    return publicKeyFromBytes(value);
  }
  if ('bytes' in value && 'base58' in value) {
    return value;
  }
  throw new Error(`Cannot convert to PublicKey: ${value}`);
}
