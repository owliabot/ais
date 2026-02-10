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
// Extensions (strict schemas)
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Reserved extensibility slot.
 *
 * All AIS 0.0.2 core objects are strict: unknown fields MUST be rejected.
 * If an implementation needs to attach extra metadata, it MUST do so under
 * an `extensions` object.
 */
export const ExtensionsSchema = z.record(z.unknown()).optional();

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
  extensions: ExtensionsSchema,
}).strict();

/**
 * Token amount (AIS 0.0.2)
 *
 * Human-facing decimal string (no exponent). Binding to an `asset` is defined
 * by the Param definition's `asset_ref` field (not by the value itself).
 */
export const TokenAmountSchema = z
  .string()
  .min(1)
  .refine((v) => v === 'max' || /^\d+(\.\d+)?$/.test(v), {
    message: 'token_amount must be a decimal string (e.g., "1.23") or "max"',
  });

// ═══════════════════════════════════════════════════════════════════════════════
// AIS Type System
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Valid AIS parameter types
 */
const EvmIntTypeSchema = z
  .string()
  .regex(/^(u?int)\d{1,3}$/)
  .refine((t) => {
    const isUint = t.startsWith('uint');
    const bitsStr = t.slice(isUint ? 4 : 3);
    const bits = Number(bitsStr);
    return Number.isInteger(bits) && bits >= 8 && bits <= 256 && bits % 8 === 0;
  }, { message: 'int/uint types must be a multiple of 8 bits (8..256), e.g., uint24, int128' });

const EvmFixedBytesTypeSchema = z
  .string()
  .regex(/^bytes\d{1,2}$/)
  .refine((t) => {
    const n = Number(t.slice('bytes'.length));
    return Number.isInteger(n) && n >= 1 && n <= 32;
  }, { message: 'Fixed bytes types must be bytes1..bytes32' });

export const AISTypeSchema = z.union([
  // Basic types
  z.enum(['address', 'bool', 'string', 'bytes', 'float']),
  // EVM scalar types
  EvmIntTypeSchema,
  EvmFixedBytesTypeSchema,
  // Composite types
  z.enum(['asset', 'token_amount']),
  // Array and tuple types: array<T>, tuple<T1,T2,...>
  z.string().regex(/^(array|tuple)<.+>$/),
]);

// ═══════════════════════════════════════════════════════════════════════════════
// ValueRef (AIS 0.0.2)
// ═══════════════════════════════════════════════════════════════════════════════

export const DetectKindSchema = z.enum(['choose_one', 'best_quote', 'best_path', 'protocol_specific']);

export const DetectSchema = z.object({
  kind: DetectKindSchema,
  provider: z.string().optional(),
  candidates: z.array(z.unknown()).optional(),
  constraints: z.record(z.unknown()).optional(),
  requires_capabilities: z.array(z.string()).optional(),
  extensions: ExtensionsSchema,
}).strict();

// Inferred types
export type ChainId = z.infer<typeof ChainIdSchema>;
export type Asset = z.infer<typeof AssetSchema>;
export type TokenAmount = z.infer<typeof TokenAmountSchema>;
export type AISType = z.infer<typeof AISTypeSchema>;
export type Detect = z.infer<typeof DetectSchema>;
export type Extensions = z.infer<typeof ExtensionsSchema>;

export type ValueRef =
  | { lit: unknown }
  | { ref: string }
  | { cel: string }
  | { detect: Detect }
  | { object: Record<string, ValueRef> }
  | { array: ValueRef[] };

export const ValueRefSchema: z.ZodType<ValueRef> = z.lazy(() =>
  z.union([
    z.object({ lit: z.unknown() }).strict(),
    z.object({ ref: z.string() }).strict(),
    z.object({ cel: z.string() }).strict(),
    z.object({ detect: DetectSchema }).strict(),
    z.object({ object: z.record(ValueRefSchema) }).strict(),
    z.object({ array: z.array(ValueRefSchema) }).strict(),
  ])
) as z.ZodType<ValueRef>;
