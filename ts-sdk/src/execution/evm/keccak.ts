/**
 * Keccak-256 hash function
 * Uses ethers for reliable implementation
 */

import { keccak256 as ethersKeccak256, toUtf8Bytes } from 'ethers';

/**
 * Compute Keccak-256 hash and return as hex string with 0x prefix
 */
export function keccak256(input: string | Uint8Array): string {
  if (typeof input === 'string') {
    return ethersKeccak256(toUtf8Bytes(input));
  }
  return ethersKeccak256(input);
}
