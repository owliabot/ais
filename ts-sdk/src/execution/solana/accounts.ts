/**
 * Solana account resolution from AIS specs
 */

import { toPublicKey } from './pubkey.js';
import { findProgramAddressSync } from './pda.js';
import { getAssociatedTokenAddressSync } from './ata.js';
import { 
  TOKEN_PROGRAM_ID, 
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  RENT_SYSVAR_ID,
  CLOCK_SYSVAR_ID,
} from './constants.js';
import type { PublicKey, AccountMeta, SolanaAccountSpec, SolanaResolverContext } from './types.js';

/**
 * System account constants
 */
const SYSTEM_ACCOUNTS: Record<string, string> = {
  'system_program': SYSTEM_PROGRAM_ID,
  'token_program': TOKEN_PROGRAM_ID,
  'token_2022_program': TOKEN_2022_PROGRAM_ID,
  'associated_token_program': ASSOCIATED_TOKEN_PROGRAM_ID,
  'rent': RENT_SYSVAR_ID,
  'clock': CLOCK_SYSVAR_ID,
};

/**
 * Sysvar accounts
 */
const SYSVAR_ACCOUNTS: Record<string, string> = {
  'rent': RENT_SYSVAR_ID,
  'clock': CLOCK_SYSVAR_ID,
  'slot_hashes': 'SysvarS1otHashes111111111111111111111111111',
  'slot_history': 'SysvarS1otHistory11111111111111111111111111',
  'stake_history': 'SysvarStakeHistory1111111111111111111111111',
  'instructions': 'Sysvar1nstructions1111111111111111111111111',
  'recent_blockhashes': 'SysvarRecentB1telefonists111111111111111111',
  'epoch_schedule': 'SysvarEpochScheworthy1111111111111111111111',
  'fees': 'SysvarFees111111111111111111111111111111111',
};

/**
 * Resolve a value expression from context
 */
function resolveExpression(expr: string, ctx: SolanaResolverContext): string {
  const parts = expr.split('.');
  
  // Handle different prefixes
  const prefix = parts[0];
  const path = parts.slice(1);

  switch (prefix) {
    case 'wallet':
    case 'ctx':
      if (path[0] === 'wallet_address' || parts.length === 1) {
        return ctx.walletAddress;
      }
      break;
      
    case 'params':
      return resolveNestedPath(ctx.params, path);
      
    case 'contracts':
      if (path.length === 1 && ctx.contracts[path[0]]) {
        return ctx.contracts[path[0]];
      }
      break;
      
    case 'calculated':
      return resolveNestedPath(ctx.calculated, path);
      
    case 'query':
      if (path.length >= 2) {
        const queryId = path[0];
        const queryPath = path.slice(1);
        if (ctx.query[queryId]) {
          return resolveNestedPath(ctx.query[queryId], queryPath);
        }
      }
      break;
      
    case 'constant':
      // constant:ADDRESS format
      return parts.slice(1).join('.');
      
    case 'system':
      // system:token_program format
      const systemKey = path[0];
      if (SYSTEM_ACCOUNTS[systemKey]) {
        return SYSTEM_ACCOUNTS[systemKey];
      }
      throw new Error(`Unknown system account: ${systemKey}`);
      
    case 'sysvar':
      // sysvar:rent format
      const sysvarKey = path[0];
      if (SYSVAR_ACCOUNTS[sysvarKey]) {
        return SYSVAR_ACCOUNTS[sysvarKey];
      }
      throw new Error(`Unknown sysvar: ${sysvarKey}`);
  }

  // Check if it's a direct contract reference
  if (ctx.contracts[expr]) {
    return ctx.contracts[expr];
  }

  throw new Error(`Cannot resolve expression: ${expr}`);
}

/**
 * Resolve nested path in an object
 */
function resolveNestedPath(obj: Record<string, unknown>, path: string[]): string {
  let current: unknown = obj;
  
  for (const key of path) {
    if (current === null || current === undefined) {
      throw new Error(`Cannot resolve path: ${path.join('.')}`);
    }
    if (typeof current === 'object') {
      current = (current as Record<string, unknown>)[key];
    } else {
      throw new Error(`Cannot access ${key} on non-object`);
    }
  }

  if (typeof current === 'string') {
    return current;
  }
  if (typeof current === 'number' || typeof current === 'bigint') {
    return current.toString();
  }

  throw new Error(`Resolved value is not a string: ${typeof current}`);
}

/**
 * Resolve a single account from AIS spec
 */
export function resolveAccount(
  spec: SolanaAccountSpec,
  ctx: SolanaResolverContext
): AccountMeta {
  let pubkey: PublicKey;

  // Handle derived accounts (ATA or PDA)
  if (spec.derived) {
    if (spec.derived === 'ata') {
      // ATA derivation requires wallet and mint
      const wallet = spec.wallet 
        ? resolveExpression(spec.wallet, ctx)
        : ctx.walletAddress;
      const mint = spec.mint 
        ? resolveExpression(spec.mint, ctx)
        : resolveExpression(spec.source, ctx);
      
      pubkey = getAssociatedTokenAddressSync(wallet, mint);
    } else if (spec.derived === 'pda') {
      // PDA derivation requires seeds and program
      if (!spec.seeds || !spec.program) {
        throw new Error(`PDA derivation requires seeds and program: ${spec.name}`);
      }
      
      const seeds = spec.seeds.map(seed => {
        // Check if seed is a literal string (no dots) or an expression
        if (!seed.includes('.') && !seed.startsWith('params') && !seed.startsWith('ctx')) {
          return seed;  // Literal string seed
        }
        return resolveExpression(seed, ctx);
      });
      
      const programId = spec.program.startsWith('constant:')
        ? spec.program.slice(9)
        : resolveExpression(spec.program, ctx);
      
      const [derivedAddress] = findProgramAddressSync(
        seeds.map(s => typeof s === 'string' ? s : new TextEncoder().encode(String(s))),
        programId
      );
      pubkey = derivedAddress;
    } else {
      throw new Error(`Unknown derived type: ${spec.derived}`);
    }
  } else {
    // Direct address resolution
    const source = spec.source;
    
    // Handle special sources
    if (source === 'wallet') {
      pubkey = toPublicKey(ctx.walletAddress);
    } else if (source.startsWith('system:')) {
      const systemKey = source.slice(7);
      if (!SYSTEM_ACCOUNTS[systemKey]) {
        throw new Error(`Unknown system account: ${systemKey}`);
      }
      pubkey = toPublicKey(SYSTEM_ACCOUNTS[systemKey]);
    } else if (source.startsWith('sysvar:')) {
      const sysvarKey = source.slice(7);
      if (!SYSVAR_ACCOUNTS[sysvarKey]) {
        throw new Error(`Unknown sysvar: ${sysvarKey}`);
      }
      pubkey = toPublicKey(SYSVAR_ACCOUNTS[sysvarKey]);
    } else if (source.startsWith('constant:')) {
      pubkey = toPublicKey(source.slice(9));
    } else {
      // Expression-based source
      const address = resolveExpression(source, ctx);
      pubkey = toPublicKey(address);
    }
  }

  return {
    pubkey,
    isSigner: spec.signer,
    isWritable: spec.writable,
  };
}

/**
 * Resolve all accounts from an AIS spec
 */
export function resolveAccounts(
  specs: SolanaAccountSpec[],
  ctx: SolanaResolverContext
): AccountMeta[] {
  return specs.map(spec => resolveAccount(spec, ctx));
}

/**
 * Create account resolver from context
 */
export function createAccountResolver(ctx: SolanaResolverContext) {
  return {
    resolve: (expr: string) => resolveExpression(expr, ctx),
    resolveAccount: (spec: SolanaAccountSpec) => resolveAccount(spec, ctx),
    resolveAccounts: (specs: SolanaAccountSpec[]) => resolveAccounts(specs, ctx),
  };
}
