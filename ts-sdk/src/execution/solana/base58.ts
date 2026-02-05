/**
 * Base58 encoding/decoding (Bitcoin alphabet)
 * Zero-dependency implementation for Solana addresses
 */

const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const ALPHABET_MAP = new Map<string, number>();
for (let i = 0; i < ALPHABET.length; i++) {
  ALPHABET_MAP.set(ALPHABET[i], i);
}

/**
 * Encode bytes to Base58 string
 */
export function base58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return '';

  // Count leading zeros
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) {
    zeros++;
  }

  // Allocate enough space in big-endian base58 representation
  const size = Math.ceil((bytes.length * 138) / 100) + 1;
  const b58 = new Uint8Array(size);

  // Process the bytes
  let length = 0;
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i];
    let j = 0;
    for (let k = size - 1; k >= 0 && (carry !== 0 || j < length); k--, j++) {
      carry += 256 * b58[k];
      b58[k] = carry % 58;
      carry = Math.floor(carry / 58);
    }
    length = j;
  }

  // Skip leading zeros in base58 result
  let start = size - length;
  while (start < size && b58[start] === 0) {
    start++;
  }

  // Translate to base58 string
  let result = ALPHABET[0].repeat(zeros);
  for (let i = start; i < size; i++) {
    result += ALPHABET[b58[i]];
  }

  return result;
}

/**
 * Decode Base58 string to bytes
 */
export function base58Decode(str: string): Uint8Array {
  if (str.length === 0) return new Uint8Array(0);

  // Count leading '1's (zeros in base58)
  let zeros = 0;
  while (zeros < str.length && str[zeros] === '1') {
    zeros++;
  }

  // Allocate enough space
  const size = Math.ceil((str.length * 733) / 1000) + 1;
  const bytes = new Uint8Array(size);

  // Process the characters
  let length = 0;
  for (let i = zeros; i < str.length; i++) {
    const value = ALPHABET_MAP.get(str[i]);
    if (value === undefined) {
      throw new Error(`Invalid base58 character: ${str[i]}`);
    }

    let carry = value;
    let j = 0;
    for (let k = size - 1; k >= 0 && (carry !== 0 || j < length); k--, j++) {
      carry += 58 * bytes[k];
      bytes[k] = carry % 256;
      carry = Math.floor(carry / 256);
    }
    length = j;
  }

  // Skip leading zeros in byte result
  let start = size - length;
  while (start < size && bytes[start] === 0) {
    start++;
  }

  // Build result with leading zeros
  const result = new Uint8Array(zeros + (size - start));
  for (let i = start; i < size; i++) {
    result[zeros + (i - start)] = bytes[i];
  }

  return result;
}

/**
 * Check if a string is valid Base58
 */
export function isValidBase58(str: string): boolean {
  if (str.length === 0) return false;
  for (const char of str) {
    if (!ALPHABET_MAP.has(char)) {
      return false;
    }
  }
  return true;
}

/**
 * Check if a string is a valid Solana public key (32 bytes Base58)
 */
export function isValidPublicKey(str: string): boolean {
  if (!isValidBase58(str)) return false;
  try {
    const decoded = base58Decode(str);
    return decoded.length === 32;
  } catch {
    return false;
  }
}
