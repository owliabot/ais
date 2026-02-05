/**
 * Solana execution module for AIS SDK
 * 
 * Provides instruction building, account resolution, and serialization
 * for Solana programs defined in AIS specs.
 */

// Constants
export * from './constants.js';

// Types
export type {
  PublicKey,
  AccountMeta,
  TransactionInstruction,
  SolanaInstructionResult,
  SolanaAccountSpec,
  SolanaExecutionSpec,
  SolanaResolverContext,
  SolanaBuildOptions,
} from './types.js';

// Base58 encoding
export {
  base58Encode,
  base58Decode,
  isValidBase58,
  isValidPublicKey,
} from './base58.js';

// Public key utilities
export {
  publicKeyFromBase58,
  publicKeyFromBytes,
  publicKeyFromHex,
  publicKeysEqual,
  publicKeyToHex,
  isPublicKeyLike,
  toPublicKey,
} from './pubkey.js';

// SHA-256 hashing
export {
  sha256,
  sha256Sync,
} from './sha256.js';

// PDA derivation
export {
  findProgramAddressSync,
  findProgramAddress,
  createProgramAddressSync,
  derivePdaFromSpec,
} from './pda.js';

// ATA derivation
export {
  getAssociatedTokenAddressSync,
  getAssociatedTokenAddress,
  createAssociatedTokenAccountInstruction,
  createAssociatedTokenAccountIdempotentInstruction,
  deriveAtaFromSpec,
} from './ata.js';

// Borsh serialization
export {
  BorshWriter,
  BorshReader,
  serializeSplTransfer,
  serializeSplTransferChecked,
  serializeSplApprove,
} from './borsh.js';

// Account resolution
export {
  resolveAccount,
  resolveAccounts,
  createAccountResolver,
} from './accounts.js';

// Instruction building
export {
  buildSolanaInstruction,
  buildSplTransfer,
  buildSplApprove,
  buildSplTransferWithAtaCreation,
  serializeInstruction,
} from './builder.js';
