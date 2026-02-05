/**
 * Solana instruction builder from AIS specs
 */

import { toPublicKey } from './pubkey.js';
import { resolveAccounts, createAccountResolver } from './accounts.js';
import { BorshWriter, serializeSplTransfer, serializeSplTransferChecked, serializeSplApprove } from './borsh.js';
import { getAssociatedTokenAddressSync, createAssociatedTokenAccountIdempotentInstruction } from './ata.js';
import { TOKEN_PROGRAM_ID, SPL_TOKEN_INSTRUCTIONS, COMPUTE_UNITS } from './constants.js';
import type { 
  PublicKey,
  TransactionInstruction, 
  SolanaInstructionResult, 
  SolanaExecutionSpec, 
  SolanaResolverContext,
  SolanaBuildOptions,
} from './types.js';

/**
 * Parse discriminator from hex string
 */
function parseDiscriminator(discriminator: string): Uint8Array {
  const hex = discriminator.startsWith('0x') ? discriminator.slice(2) : discriminator;
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/**
 * Resolve a mapping value to its concrete value
 */
function resolveMappingValue(
  value: string,
  ctx: SolanaResolverContext,
  resolver: ReturnType<typeof createAccountResolver>
): bigint | number | string | Uint8Array {
  // Check for literal values
  if (/^\d+$/.test(value)) {
    return BigInt(value);
  }
  if (value.startsWith('0x')) {
    // Hex bytes
    const hex = value.slice(2);
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    }
    return bytes;
  }

  // Resolve expression
  const resolved = resolver.resolve(value);
  
  // Convert to appropriate type
  if (/^\d+$/.test(resolved)) {
    return BigInt(resolved);
  }
  return resolved;
}

/**
 * Build instruction data from mapping and instruction name
 */
function buildInstructionData(
  instructionName: string,
  discriminator: string | undefined,
  mapping: Record<string, string>,
  ctx: SolanaResolverContext,
  resolver: ReturnType<typeof createAccountResolver>
): Uint8Array {
  const writer = new BorshWriter();

  // Write discriminator if provided
  if (discriminator) {
    const discBytes = parseDiscriminator(discriminator);
    writer.writeBytes(discBytes);
  }

  // Serialize each mapped value in order
  // Note: Order matters in Borsh! The spec should define mapping in correct order
  for (const [_key, valueExpr] of Object.entries(mapping)) {
    const value = resolveMappingValue(valueExpr, ctx, resolver);
    
    if (typeof value === 'bigint') {
      // Assume u64 for bigint values (most common in Solana)
      writer.writeU64(value);
    } else if (typeof value === 'number') {
      // Assume u32 for number values
      writer.writeU32(value);
    } else if (value instanceof Uint8Array) {
      // Raw bytes
      writer.writeBytes(value);
    } else if (typeof value === 'string') {
      // Check if it's a public key
      if (value.length >= 32 && value.length <= 44) {
        try {
          const pubkey = toPublicKey(value);
          writer.writePublicKey(pubkey.bytes);
        } catch {
          // Not a pubkey, write as string
          writer.writeString(value);
        }
      } else {
        writer.writeString(value);
      }
    }
  }

  return writer.toBytes();
}

/**
 * Build a Solana instruction from AIS execution spec
 */
export function buildSolanaInstruction(
  spec: SolanaExecutionSpec,
  ctx: SolanaResolverContext,
  options: SolanaBuildOptions = { chain: 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp' }
): SolanaInstructionResult {
  const resolver = createAccountResolver(ctx);
  
  // Resolve program ID
  const programId = spec.program.startsWith('constant:')
    ? toPublicKey(spec.program.slice(9))
    : toPublicKey(resolver.resolve(spec.program));

  // Resolve all accounts
  const keys = resolveAccounts(spec.accounts, ctx);

  // Build instruction data
  const data = buildInstructionData(
    spec.instruction,
    spec.discriminator,
    spec.mapping,
    ctx,
    resolver
  );

  const result: SolanaInstructionResult = {
    instruction: {
      programId,
      keys,
      data,
    },
    computeUnits: spec.compute_units,
    lookupTables: spec.lookup_tables,
  };

  return result;
}

/**
 * Build SPL Token transfer instruction
 * Convenience function for the most common operation
 */
export function buildSplTransfer(
  source: string | PublicKey,
  destination: string | PublicKey,
  owner: string | PublicKey,
  amount: bigint,
  options: { checked?: boolean; decimals?: number; mint?: string | PublicKey } = {}
): TransactionInstruction {
  const sourcePk = typeof source === 'string' ? toPublicKey(source) : source;
  const destPk = typeof destination === 'string' ? toPublicKey(destination) : destination;
  const ownerPk = typeof owner === 'string' ? toPublicKey(owner) : owner;
  const programId = toPublicKey(TOKEN_PROGRAM_ID);

  if (options.checked && options.decimals !== undefined && options.mint) {
    // TransferChecked
    const mintPk = typeof options.mint === 'string' ? toPublicKey(options.mint) : options.mint;
    return {
      programId,
      keys: [
        { pubkey: sourcePk, isSigner: false, isWritable: true },
        { pubkey: mintPk, isSigner: false, isWritable: false },
        { pubkey: destPk, isSigner: false, isWritable: true },
        { pubkey: ownerPk, isSigner: true, isWritable: false },
      ],
      data: serializeSplTransferChecked(amount, options.decimals),
    };
  }

  // Simple Transfer
  return {
    programId,
    keys: [
      { pubkey: sourcePk, isSigner: false, isWritable: true },
      { pubkey: destPk, isSigner: false, isWritable: true },
      { pubkey: ownerPk, isSigner: true, isWritable: false },
    ],
    data: serializeSplTransfer(amount),
  };
}

/**
 * Build SPL Token approve instruction
 */
export function buildSplApprove(
  source: string | PublicKey,
  delegate: string | PublicKey,
  owner: string | PublicKey,
  amount: bigint
): TransactionInstruction {
  const sourcePk = typeof source === 'string' ? toPublicKey(source) : source;
  const delegatePk = typeof delegate === 'string' ? toPublicKey(delegate) : delegate;
  const ownerPk = typeof owner === 'string' ? toPublicKey(owner) : owner;
  const programId = toPublicKey(TOKEN_PROGRAM_ID);

  return {
    programId,
    keys: [
      { pubkey: sourcePk, isSigner: false, isWritable: true },
      { pubkey: delegatePk, isSigner: false, isWritable: false },
      { pubkey: ownerPk, isSigner: true, isWritable: false },
    ],
    data: serializeSplApprove(amount),
  };
}

/**
 * Build a complete SPL Token transfer with ATA creation if needed
 */
export function buildSplTransferWithAtaCreation(
  wallet: string,
  mint: string,
  recipient: string,
  amount: bigint,
  options: { checked?: boolean; decimals?: number } = {}
): SolanaInstructionResult {
  const walletPk = toPublicKey(wallet);
  const mintPk = toPublicKey(mint);
  
  // Derive ATAs
  const sourceAta = getAssociatedTokenAddressSync(wallet, mint);
  const destAta = getAssociatedTokenAddressSync(recipient, mint);

  // Build pre-instructions (create destination ATA if needed)
  const preInstructions: TransactionInstruction[] = [
    createAssociatedTokenAccountIdempotentInstruction(
      wallet,      // payer
      destAta.base58,
      recipient,   // owner
      mint
    ),
  ];

  // Build transfer instruction
  const transferIx = buildSplTransfer(
    sourceAta,
    destAta,
    walletPk,
    amount,
    { checked: options.checked, decimals: options.decimals, mint: mintPk }
  );

  return {
    instruction: transferIx,
    preInstructions,
    computeUnits: COMPUTE_UNITS.CREATE_ATA + COMPUTE_UNITS.TRANSFER,
  };
}

/**
 * Serialize instruction to bytes for transaction building
 */
export function serializeInstruction(ix: TransactionInstruction): {
  programIdIndex: number;
  accountIndices: number[];
  data: Uint8Array;
} {
  // Note: This returns indices that need to be resolved against a transaction's
  // account list. For actual transaction building, use @solana/web3.js.
  return {
    programIdIndex: -1,  // Placeholder - needs transaction context
    accountIndices: [],  // Placeholder - needs transaction context
    data: ix.data,
  };
}
