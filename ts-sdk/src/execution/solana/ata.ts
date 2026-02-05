/**
 * Associated Token Account (ATA) utilities
 * 
 * ATAs are deterministic token accounts for a given wallet + mint combination.
 * They are PDAs derived from [wallet, TOKEN_PROGRAM, mint] using the ATA program.
 */

import { findProgramAddressSync } from './pda.js';
import { toPublicKey } from './pubkey.js';
import { 
  TOKEN_PROGRAM_ID, 
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  RENT_SYSVAR_ID,
} from './constants.js';
import type { PublicKey, TransactionInstruction, AccountMeta } from './types.js';

/**
 * Derive the Associated Token Account address for a wallet and mint
 * 
 * @param wallet - Owner wallet public key
 * @param mint - Token mint public key
 * @param tokenProgramId - Token program (default: TOKEN_PROGRAM_ID)
 * @returns The ATA address
 */
export function getAssociatedTokenAddressSync(
  wallet: PublicKey | string,
  mint: PublicKey | string,
  tokenProgramId: string = TOKEN_PROGRAM_ID
): PublicKey {
  const walletPk = typeof wallet === 'string' ? toPublicKey(wallet) : wallet;
  const mintPk = typeof mint === 'string' ? toPublicKey(mint) : mint;
  const tokenProgramPk = toPublicKey(tokenProgramId);

  const [address] = findProgramAddressSync(
    [walletPk.bytes, tokenProgramPk.bytes, mintPk.bytes],
    ASSOCIATED_TOKEN_PROGRAM_ID
  );

  return address;
}

/**
 * Async version of getAssociatedTokenAddressSync
 */
export async function getAssociatedTokenAddress(
  wallet: PublicKey | string,
  mint: PublicKey | string,
  tokenProgramId: string = TOKEN_PROGRAM_ID
): Promise<PublicKey> {
  return getAssociatedTokenAddressSync(wallet, mint, tokenProgramId);
}

/**
 * Create instruction to create an Associated Token Account
 * 
 * @param payer - Account paying for creation (usually the wallet)
 * @param associatedToken - The ATA address to create
 * @param owner - Owner of the new ATA
 * @param mint - Token mint
 * @param tokenProgramId - Token program (TOKEN_PROGRAM_ID or TOKEN_2022_PROGRAM_ID)
 */
export function createAssociatedTokenAccountInstruction(
  payer: PublicKey | string,
  associatedToken: PublicKey | string,
  owner: PublicKey | string,
  mint: PublicKey | string,
  tokenProgramId: string = TOKEN_PROGRAM_ID
): TransactionInstruction {
  const payerPk = typeof payer === 'string' ? toPublicKey(payer) : payer;
  const ataPk = typeof associatedToken === 'string' ? toPublicKey(associatedToken) : associatedToken;
  const ownerPk = typeof owner === 'string' ? toPublicKey(owner) : owner;
  const mintPk = typeof mint === 'string' ? toPublicKey(mint) : mint;
  const tokenProgramPk = toPublicKey(tokenProgramId);
  const systemProgramPk = toPublicKey(SYSTEM_PROGRAM_ID);
  const ataProgramPk = toPublicKey(ASSOCIATED_TOKEN_PROGRAM_ID);

  const keys: AccountMeta[] = [
    { pubkey: payerPk, isSigner: true, isWritable: true },
    { pubkey: ataPk, isSigner: false, isWritable: true },
    { pubkey: ownerPk, isSigner: false, isWritable: false },
    { pubkey: mintPk, isSigner: false, isWritable: false },
    { pubkey: systemProgramPk, isSigner: false, isWritable: false },
    { pubkey: tokenProgramPk, isSigner: false, isWritable: false },
  ];

  // ATA program instruction 0 = Create
  return {
    programId: ataProgramPk,
    keys,
    data: new Uint8Array([0]),  // Create instruction = 0
  };
}

/**
 * Create instruction to create an ATA idempotently (won't fail if exists)
 * Uses instruction index 1 which creates or returns existing
 */
export function createAssociatedTokenAccountIdempotentInstruction(
  payer: PublicKey | string,
  associatedToken: PublicKey | string,
  owner: PublicKey | string,
  mint: PublicKey | string,
  tokenProgramId: string = TOKEN_PROGRAM_ID
): TransactionInstruction {
  const payerPk = typeof payer === 'string' ? toPublicKey(payer) : payer;
  const ataPk = typeof associatedToken === 'string' ? toPublicKey(associatedToken) : associatedToken;
  const ownerPk = typeof owner === 'string' ? toPublicKey(owner) : owner;
  const mintPk = typeof mint === 'string' ? toPublicKey(mint) : mint;
  const tokenProgramPk = toPublicKey(tokenProgramId);
  const systemProgramPk = toPublicKey(SYSTEM_PROGRAM_ID);
  const ataProgramPk = toPublicKey(ASSOCIATED_TOKEN_PROGRAM_ID);

  const keys: AccountMeta[] = [
    { pubkey: payerPk, isSigner: true, isWritable: true },
    { pubkey: ataPk, isSigner: false, isWritable: true },
    { pubkey: ownerPk, isSigner: false, isWritable: false },
    { pubkey: mintPk, isSigner: false, isWritable: false },
    { pubkey: systemProgramPk, isSigner: false, isWritable: false },
    { pubkey: tokenProgramPk, isSigner: false, isWritable: false },
  ];

  // Idempotent create instruction = 1
  return {
    programId: ataProgramPk,
    keys,
    data: new Uint8Array([1]),
  };
}

/**
 * Derive ATA from AIS spec with wallet and mint expressions
 * 
 * @param walletExpr - Wallet expression (e.g., "ctx.wallet_address")
 * @param mintExpr - Mint expression (e.g., "params.token.address")
 * @param resolveValue - Function to resolve expressions
 * @param tokenProgramId - Token program (optional)
 */
export function deriveAtaFromSpec(
  walletExpr: string,
  mintExpr: string,
  resolveValue: (expr: string) => string,
  tokenProgramId: string = TOKEN_PROGRAM_ID
): PublicKey {
  const wallet = resolveValue(walletExpr);
  const mint = resolveValue(mintExpr);
  return getAssociatedTokenAddressSync(wallet, mint, tokenProgramId);
}
