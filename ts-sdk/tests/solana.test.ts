/**
 * Tests for Solana execution module
 */

import { describe, it, expect } from 'vitest';
import {
  base58Encode,
  base58Decode,
  isValidBase58,
  isValidPublicKey,
  publicKeyFromBase58,
  publicKeyFromBytes,
  toPublicKey,
  sha256Sync,
  findProgramAddressSync,
  getAssociatedTokenAddressSync,
  BorshWriter,
  BorshReader,
  serializeSplTransfer,
  serializeSplTransferChecked,
  buildSplTransfer,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
} from '../src/execution/solana/index.js';

describe('Base58', () => {
  it('should encode bytes to base58', () => {
    const bytes = new Uint8Array([0, 0, 0, 1]);
    const encoded = base58Encode(bytes);
    expect(encoded).toBe('1112');
  });

  it('should decode base58 to bytes', () => {
    const decoded = base58Decode('1112');
    expect(decoded).toEqual(new Uint8Array([0, 0, 0, 1]));
  });

  it('should roundtrip encode/decode', () => {
    const original = new Uint8Array(32).fill(42);
    const encoded = base58Encode(original);
    const decoded = base58Decode(encoded);
    expect(decoded).toEqual(original);
  });

  it('should validate base58 strings', () => {
    expect(isValidBase58('123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz')).toBe(true);
    expect(isValidBase58('0OIl')).toBe(false); // Invalid chars
    expect(isValidBase58('')).toBe(false);
  });

  it('should validate public keys', () => {
    expect(isValidPublicKey(TOKEN_PROGRAM_ID)).toBe(true);
    expect(isValidPublicKey('invalid')).toBe(false);
    expect(isValidPublicKey(SYSTEM_PROGRAM_ID)).toBe(true);
  });
});

describe('PublicKey', () => {
  it('should create from base58', () => {
    const pk = publicKeyFromBase58(TOKEN_PROGRAM_ID);
    expect(pk.base58).toBe(TOKEN_PROGRAM_ID);
    expect(pk.bytes.length).toBe(32);
  });

  it('should create from bytes', () => {
    const bytes = new Uint8Array(32).fill(1);
    const pk = publicKeyFromBytes(bytes);
    expect(pk.bytes).toEqual(bytes);
    expect(pk.base58.length).toBeGreaterThan(0);
  });

  it('should convert using toPublicKey', () => {
    // From string
    const pk1 = toPublicKey(TOKEN_PROGRAM_ID);
    expect(pk1.base58).toBe(TOKEN_PROGRAM_ID);

    // From bytes
    const bytes = new Uint8Array(32).fill(2);
    const pk2 = toPublicKey(bytes);
    expect(pk2.bytes).toEqual(bytes);

    // From PublicKey
    const pk3 = toPublicKey(pk1);
    expect(pk3.base58).toBe(pk1.base58);
  });

  it('should throw on invalid public key', () => {
    expect(() => publicKeyFromBase58('invalid')).toThrow();
    expect(() => publicKeyFromBytes(new Uint8Array(31))).toThrow();
  });
});

describe('SHA-256', () => {
  it('should hash empty input', () => {
    const hash = sha256Sync(new Uint8Array(0));
    // SHA-256 of empty string
    expect(hash.length).toBe(32);
  });

  it('should hash known input', () => {
    const input = new TextEncoder().encode('hello');
    const hash = sha256Sync(input);
    expect(hash.length).toBe(32);
    // First few bytes of SHA-256("hello")
    expect(hash[0]).toBe(0x2c);
    expect(hash[1]).toBe(0xf2);
  });
});

describe('PDA Derivation', () => {
  it('should find program address', () => {
    const [pda, bump] = findProgramAddressSync(
      ['test'],
      TOKEN_PROGRAM_ID
    );
    expect(pda.bytes.length).toBe(32);
    expect(bump).toBeGreaterThanOrEqual(0);
    expect(bump).toBeLessThanOrEqual(255);
  });

  it('should derive same address for same seeds', () => {
    const [pda1] = findProgramAddressSync(['seed1', 'seed2'], TOKEN_PROGRAM_ID);
    const [pda2] = findProgramAddressSync(['seed1', 'seed2'], TOKEN_PROGRAM_ID);
    expect(pda1.base58).toBe(pda2.base58);
  });

  it('should derive different address for different seeds', () => {
    const [pda1] = findProgramAddressSync(['seed1'], TOKEN_PROGRAM_ID);
    const [pda2] = findProgramAddressSync(['seed2'], TOKEN_PROGRAM_ID);
    expect(pda1.base58).not.toBe(pda2.base58);
  });

  it('should accept Uint8Array seeds', () => {
    const seed = new Uint8Array([1, 2, 3, 4]);
    const [pda, bump] = findProgramAddressSync([seed], TOKEN_PROGRAM_ID);
    expect(pda.bytes.length).toBe(32);
    expect(bump).toBeDefined();
  });
});

describe('ATA Derivation', () => {
  const testWallet = 'DYw8jCTfwHNRJhhmFcbXvVDTqWMEVFBX6ZKUmG5CNSKK';
  const testMint = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'; // USDC

  it('should derive ATA address', () => {
    const ata = getAssociatedTokenAddressSync(testWallet, testMint);
    expect(ata.bytes.length).toBe(32);
    expect(ata.base58.length).toBeGreaterThan(0);
  });

  it('should derive same ATA for same wallet/mint', () => {
    const ata1 = getAssociatedTokenAddressSync(testWallet, testMint);
    const ata2 = getAssociatedTokenAddressSync(testWallet, testMint);
    expect(ata1.base58).toBe(ata2.base58);
  });

  it('should derive different ATA for different mint', () => {
    const mint2 = 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB'; // USDT
    const ata1 = getAssociatedTokenAddressSync(testWallet, testMint);
    const ata2 = getAssociatedTokenAddressSync(testWallet, mint2);
    expect(ata1.base58).not.toBe(ata2.base58);
  });
});

describe('Borsh Serialization', () => {
  it('should write and read u8', () => {
    const writer = new BorshWriter();
    writer.writeU8(42);
    const reader = new BorshReader(writer.toBytes());
    expect(reader.readU8()).toBe(42);
  });

  it('should write and read u64', () => {
    const writer = new BorshWriter();
    writer.writeU64(BigInt('1000000000000'));
    const reader = new BorshReader(writer.toBytes());
    expect(reader.readU64()).toBe(BigInt('1000000000000'));
  });

  it('should write and read string', () => {
    const writer = new BorshWriter();
    writer.writeString('hello solana');
    const reader = new BorshReader(writer.toBytes());
    expect(reader.readString()).toBe('hello solana');
  });

  it('should write and read boolean', () => {
    const writer = new BorshWriter();
    writer.writeBool(true);
    writer.writeBool(false);
    const reader = new BorshReader(writer.toBytes());
    expect(reader.readBool()).toBe(true);
    expect(reader.readBool()).toBe(false);
  });

  it('should serialize SPL transfer', () => {
    const data = serializeSplTransfer(BigInt(1000000));
    expect(data.length).toBe(9);
    expect(data[0]).toBe(3); // Transfer instruction
  });

  it('should serialize SPL transfer checked', () => {
    const data = serializeSplTransferChecked(BigInt(1000000), 6);
    expect(data.length).toBe(10);
    expect(data[0]).toBe(12); // TransferChecked instruction
    expect(data[9]).toBe(6);  // decimals
  });
});

describe('SPL Token Instructions', () => {
  const sourceAta = 'DYw8jCTfwHNRJhhmFcbXvVDTqWMEVFBX6ZKUmG5CNSKK';
  const destAta = '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
  const owner = 'BPFLoaderUpgradeab1e11111111111111111111111';

  it('should build transfer instruction', () => {
    const ix = buildSplTransfer(sourceAta, destAta, owner, BigInt(1000000));
    
    expect(ix.programId.base58).toBe(TOKEN_PROGRAM_ID);
    expect(ix.keys.length).toBe(3);
    expect(ix.keys[0].isWritable).toBe(true);
    expect(ix.keys[1].isWritable).toBe(true);
    expect(ix.keys[2].isSigner).toBe(true);
    expect(ix.data[0]).toBe(3); // Transfer instruction
  });

  it('should build transfer checked instruction', () => {
    const mint = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v';
    const ix = buildSplTransfer(sourceAta, destAta, owner, BigInt(1000000), {
      checked: true,
      decimals: 6,
      mint,
    });
    
    expect(ix.programId.base58).toBe(TOKEN_PROGRAM_ID);
    expect(ix.keys.length).toBe(4); // source, mint, dest, owner
    expect(ix.data[0]).toBe(12); // TransferChecked instruction
  });
});

describe('Constants', () => {
  it('should have valid program IDs', () => {
    expect(isValidPublicKey(TOKEN_PROGRAM_ID)).toBe(true);
    expect(isValidPublicKey(ASSOCIATED_TOKEN_PROGRAM_ID)).toBe(true);
    expect(isValidPublicKey(SYSTEM_PROGRAM_ID)).toBe(true);
  });

  it('should have correct system program ID', () => {
    // System program is all 1s (32 bytes of 0x01 in little-endian = "111...")
    expect(SYSTEM_PROGRAM_ID).toBe('11111111111111111111111111111111');
  });
});
