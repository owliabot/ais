/**
 * Common schemas shared across document types
 */
import { z } from 'zod';

export const HexAddressSchema = z
  .string()
  .regex(/^0x[a-fA-F0-9]{40}$/, 'Invalid Ethereum address');

export const AssetSchema = z.object({
  chain_id: z.number().int().positive(),
  address: HexAddressSchema,
  symbol: z.string().optional(),
  decimals: z.number().int().min(0).max(77).optional(),
});

export const TokenAmountSchema = z.object({
  asset: z.union([AssetSchema, z.string()]),
  amount: z.string(),
  human_readable: z.string().optional(),
});

// Inferred types
export type Asset = z.infer<typeof AssetSchema>;
export type TokenAmount = z.infer<typeof TokenAmountSchema>;
