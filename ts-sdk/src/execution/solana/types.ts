/**
 * Solana-specific type definitions for AIS
 */

/**
 * Represents a Solana public key (32 bytes)
 */
export interface PublicKey {
  readonly bytes: Uint8Array;
  readonly base58: string;
}

/**
 * Account metadata for transaction building
 */
export interface AccountMeta {
  pubkey: PublicKey;
  isSigner: boolean;
  isWritable: boolean;
}

/**
 * Solana instruction structure
 */
export interface TransactionInstruction {
  programId: PublicKey;
  keys: AccountMeta[];
  data: Uint8Array;
}

/**
 * Result of building a Solana instruction from AIS spec
 */
export interface SolanaInstructionResult {
  instruction: TransactionInstruction;
  computeUnits?: number;
  lookupTables?: string[];
  preInstructions?: TransactionInstruction[];  // e.g., create ATA if needed
}

/**
 * AIS account spec from solana_instruction execution type
 */
export interface SolanaAccountSpec {
  name: string;
  signer: boolean;
  writable: boolean;
  source: string;  // wallet | params.* | constant:* | derived | query.* | system:*
  derived?: 'ata' | 'pda' | null;
  seeds?: string[];  // For PDA derivation
  wallet?: string;   // For ATA derivation
  mint?: string;     // For ATA derivation
  program?: string;  // Program ID for PDA derivation
}

/**
 * AIS solana_instruction execution spec
 */
export interface SolanaExecutionSpec {
  type: 'solana_instruction';
  program: string;
  instruction: string;
  idl?: string;
  discriminator?: string;
  accounts: SolanaAccountSpec[];
  mapping: Record<string, string>;
  compute_units?: number;
  lookup_tables?: string[];
}

/**
 * Context for resolving Solana accounts and values
 */
export interface SolanaResolverContext {
  walletAddress: string;
  chainId: string;
  params: Record<string, unknown>;
  contracts: Record<string, string>;
  calculated: Record<string, unknown>;
  query: Record<string, Record<string, unknown>>;
}

/**
 * Options for building Solana instructions
 */
export interface SolanaBuildOptions {
  chain: string;
  createAtaIfNeeded?: boolean;
  computeUnitPrice?: number;  // Priority fee in microlamports
}
