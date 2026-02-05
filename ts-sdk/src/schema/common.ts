/**
 * Common schemas shared across document types
 */
import { z } from 'zod';

// ═══════════════════════════════════════════════════════════════════════════════
// Primitives
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * CAIP-2 chain identifier
 * @see https://github.com/ChainAgnostic/CAIPs/blob/main/CAIPs/caip-2.md
 */
export const ChainIdSchema = z.string().regex(
  /^(eip155|solana|cosmos|bip122|aptos|sui):[a-zA-Z0-9._-]+$/,
  'Invalid CAIP-2 chain ID (e.g., eip155:1, solana:mainnet)'
);

/**
 * Hex-encoded Ethereum address
 */
export const HexAddressSchema = z.string().regex(
  /^0x[a-fA-F0-9]{40}$/,
  'Invalid Ethereum address'
);

/**
 * Generic address (chain-native format)
 */
export const AddressSchema = z.string().min(1);

// ═══════════════════════════════════════════════════════════════════════════════
// Asset Types
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Asset composite type - token with full chain context
 * Used in action/query params
 */
export const AssetSchema = z.object({
  chain_id: ChainIdSchema,
  address: AddressSchema,
  symbol: z.string().optional(),
  decimals: z.number().int().min(0).max(77).optional(),
});

/**
 * Token amount - human-readable amount bound to an asset
 */
export const TokenAmountSchema = z.object({
  asset: z.union([AssetSchema, z.string()]),
  amount: z.string(),
  human_readable: z.string().optional(),
});

// ═══════════════════════════════════════════════════════════════════════════════
// AIS Type System
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Valid AIS parameter types
 */
export const AISTypeSchema = z.enum([
  // Basic types
  'address',
  'bool',
  'string',
  'bytes',
  'float',
  // Unsigned integers
  'uint8', 'uint16', 'uint32', 'uint64', 'uint128', 'uint256',
  // Signed integers
  'int8', 'int16', 'int32', 'int64', 'int128', 'int256',
  // Fixed bytes
  'bytes1', 'bytes2', 'bytes4', 'bytes8', 'bytes16', 'bytes32',
  // Composite types
  'asset',
  'token_amount',
]).or(
  // Array and tuple types: array<T>, tuple<T1,T2,...>
  z.string().regex(/^(array|tuple)<.+>$/)
);

// Inferred types
export type ChainId = z.infer<typeof ChainIdSchema>;
export type Asset = z.infer<typeof AssetSchema>;
export type TokenAmount = z.infer<typeof TokenAmountSchema>;
export type AISType = z.infer<typeof AISTypeSchema>;
