/**
 * Keccak-256 hash function
 * Uses js-sha3 library for reliable implementation
 */

// eslint-disable-next-line @typescript-eslint/no-require-imports
const { keccak256: keccak256Impl } = require('js-sha3');

/**
 * Compute Keccak-256 hash and return as hex string with 0x prefix
 */
export function keccak256(input: string | Uint8Array): string {
  return '0x' + keccak256Impl(input);
}
