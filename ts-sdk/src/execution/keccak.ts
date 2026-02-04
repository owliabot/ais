/**
 * Keccak-256 hash function
 * Uses js-sha3 library for reliable implementation
 */

import sha3 from 'js-sha3';
const keccak256Impl = sha3.keccak256;

/**
 * Compute Keccak-256 hash and return as hex string with 0x prefix
 */
export function keccak256(input: string | Uint8Array): string {
  return '0x' + keccak256Impl(input);
}
