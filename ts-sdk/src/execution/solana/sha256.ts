/**
 * SHA-256 implementation for Solana PDA derivation
 * Uses Web Crypto API when available, falls back to pure JS
 */

// K constants for SHA-256
const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

// Initial hash values
const H0 = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]);

function rotr(x: number, n: number): number {
  return (x >>> n) | (x << (32 - n));
}

function ch(x: number, y: number, z: number): number {
  return (x & y) ^ (~x & z);
}

function maj(x: number, y: number, z: number): number {
  return (x & y) ^ (x & z) ^ (y & z);
}

function sigma0(x: number): number {
  return rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22);
}

function sigma1(x: number): number {
  return rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25);
}

function gamma0(x: number): number {
  return rotr(x, 7) ^ rotr(x, 18) ^ (x >>> 3);
}

function gamma1(x: number): number {
  return rotr(x, 17) ^ rotr(x, 19) ^ (x >>> 10);
}

/**
 * Compute SHA-256 hash (synchronous, pure JS)
 */
export function sha256Sync(data: Uint8Array): Uint8Array {
  // Pre-processing: adding padding bits
  // Total message: data + 0x80 + zeros + 8-byte length = multiple of 64
  const bitLength = data.length * 8;
  
  // Calculate padded length (must be multiple of 64)
  // We need: data.length + 1 (0x80) + padding + 8 (length) ≡ 0 (mod 64)
  // So: padding = 64 - ((data.length + 1 + 8) % 64)
  // But if (data.length + 9) % 64 === 0, we need a full block of padding
  const remainder = (data.length + 9) % 64;
  const paddingZeros = remainder === 0 ? 0 : 64 - remainder;
  const paddedLength = data.length + 1 + paddingZeros + 8;
  
  const padded = new Uint8Array(paddedLength);
  padded.set(data);
  padded[data.length] = 0x80;
  // Zeros are already there (Uint8Array is zero-initialized)
  
  // Append length in bits as 64-bit big-endian
  const view = new DataView(padded.buffer);
  view.setBigUint64(paddedLength - 8, BigInt(bitLength), false);

  // Initialize hash values
  const h = new Uint32Array(H0);
  const w = new Uint32Array(64);

  // Process each 512-bit chunk
  for (let offset = 0; offset < paddedLength; offset += 64) {
    // Prepare message schedule
    for (let i = 0; i < 16; i++) {
      w[i] = view.getUint32(offset + i * 4, false);
    }
    for (let i = 16; i < 64; i++) {
      w[i] = (gamma1(w[i - 2]) + w[i - 7] + gamma0(w[i - 15]) + w[i - 16]) >>> 0;
    }

    // Initialize working variables
    let a = h[0], b = h[1], c = h[2], d = h[3];
    let e = h[4], f = h[5], g = h[6], hh = h[7];

    // Main loop
    for (let i = 0; i < 64; i++) {
      const t1 = (hh + sigma1(e) + ch(e, f, g) + K[i] + w[i]) >>> 0;
      const t2 = (sigma0(a) + maj(a, b, c)) >>> 0;
      hh = g;
      g = f;
      f = e;
      e = (d + t1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) >>> 0;
    }

    // Add compressed chunk to hash value
    h[0] = (h[0] + a) >>> 0;
    h[1] = (h[1] + b) >>> 0;
    h[2] = (h[2] + c) >>> 0;
    h[3] = (h[3] + d) >>> 0;
    h[4] = (h[4] + e) >>> 0;
    h[5] = (h[5] + f) >>> 0;
    h[6] = (h[6] + g) >>> 0;
    h[7] = (h[7] + hh) >>> 0;
  }

  // Produce final hash value (big-endian)
  const result = new Uint8Array(32);
  const resultView = new DataView(result.buffer);
  for (let i = 0; i < 8; i++) {
    resultView.setUint32(i * 4, h[i], false);
  }

  return result;
}

/**
 * Compute SHA-256 hash (async, uses Web Crypto when available)
 */
export async function sha256(data: Uint8Array): Promise<Uint8Array> {
  if (typeof globalThis.crypto?.subtle?.digest === 'function') {
    // Create a copy with a regular ArrayBuffer to satisfy Web Crypto API
    const buffer = new ArrayBuffer(data.length);
    new Uint8Array(buffer).set(data);
    const hash = await globalThis.crypto.subtle.digest('SHA-256', buffer);
    return new Uint8Array(hash);
  }
  return sha256Sync(data);
}
