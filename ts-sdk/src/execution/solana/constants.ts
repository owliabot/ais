/**
 * Solana Program Constants
 */

// System programs
export const SYSTEM_PROGRAM_ID = '11111111111111111111111111111111';
export const TOKEN_PROGRAM_ID = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
export const TOKEN_2022_PROGRAM_ID = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
export const ASSOCIATED_TOKEN_PROGRAM_ID = 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL';
export const RENT_SYSVAR_ID = 'SysvarRent111111111111111111111111111111111';
export const CLOCK_SYSVAR_ID = 'SysvarC1ock11111111111111111111111111111111';

// Solana mainnet genesis hash (for CAIP-2)
export const SOLANA_MAINNET_GENESIS = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp';
export const SOLANA_DEVNET_GENESIS = 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1';
export const SOLANA_TESTNET_GENESIS = '4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z';

// CAIP-2 chain IDs
export const SOLANA_MAINNET_CAIP2 = `solana:${SOLANA_MAINNET_GENESIS}`;
export const SOLANA_DEVNET_CAIP2 = `solana:${SOLANA_DEVNET_GENESIS}`;
export const SOLANA_TESTNET_CAIP2 = `solana:${SOLANA_TESTNET_GENESIS}`;

// SPL Token instruction discriminators
export const SPL_TOKEN_INSTRUCTIONS = {
  InitializeMint: 0,
  InitializeAccount: 1,
  InitializeMultisig: 2,
  Transfer: 3,
  Approve: 4,
  Revoke: 5,
  SetAuthority: 6,
  MintTo: 7,
  Burn: 8,
  CloseAccount: 9,
  FreezeAccount: 10,
  ThawAccount: 11,
  TransferChecked: 12,
  ApproveChecked: 13,
  MintToChecked: 14,
  BurnChecked: 15,
  InitializeAccount2: 16,
  SyncNative: 17,
  InitializeAccount3: 18,
  InitializeMultisig2: 19,
  InitializeMint2: 20,
} as const;

// Max compute units for common operations
export const COMPUTE_UNITS = {
  TRANSFER: 10_000,
  TRANSFER_CHECKED: 12_000,
  CREATE_ATA: 30_000,
  APPROVE: 5_000,
} as const;
