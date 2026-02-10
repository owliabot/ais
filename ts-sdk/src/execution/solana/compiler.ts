/**
 * Solana compiler (AIS 0.0.2)
 *
 * Compiles `solana_instruction` execution specs into a `TransactionInstruction`.
 *
 * This module performs no network IO.
 */

import type { SolanaInstruction } from '../../schema/index.js';
import type { ResolverContext } from '../../resolver/index.js';
import { evaluateValueRef, evaluateValueRefAsync, ValueRefEvalError } from '../../resolver/index.js';
import type { DetectResolver } from '../../resolver/value-ref.js';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  createApproveInstruction,
  createAssociatedTokenAccountIdempotentInstruction,
  createTransferCheckedInstruction,
  createTransferInstruction,
} from '@solana/spl-token';
import type {
  SolanaInstructionCompilerContext,
} from './registry.js';
import { SolanaInstructionCompilerRegistry } from './registry.js';

export interface CompileSolanaOptions {
  chain: string; // solana:<genesis-hash>
  /** Per-node params (shadows runtime.params during compilation) */
  params?: Record<string, unknown>;
  /** Optional registry for protocol-specific instruction compilers */
  compiler_registry?: SolanaInstructionCompilerRegistry;
  /** Optional detect resolver (may be async) */
  detect?: DetectResolver;
}

export interface CompiledSolanaInstructionRequest {
  kind: 'solana_instruction';
  chain: string;
  programId: PublicKey;
  instruction: string;
  tx: TransactionInstruction;
  computeUnits?: number;
  lookupTables?: string[];
}

export class SolanaCompileError extends Error {
  constructor(message: string, public readonly details?: unknown) {
    super(message);
    this.name = 'SolanaCompileError';
  }
}

export function createDefaultSolanaInstructionCompilerRegistry(): SolanaInstructionCompilerRegistry {
  return getDefaultRegistry().clone();
}

export function compileSolanaInstruction(
  execution: SolanaInstruction,
  ctx: ResolverContext,
  options: CompileSolanaOptions
): CompiledSolanaInstructionRequest {
  const root_overrides = options.params ? { params: options.params } : undefined;

  const declaredProgram = parsePubkey(evalString(execution.program, ctx, root_overrides, 'program'), 'program');

  const accounts = execution.accounts.map((a) => ({
    name: a.name,
    pubkey: parsePubkey(evalString(a.pubkey, ctx, root_overrides, `accounts.${a.name}.pubkey`), `accounts.${a.name}.pubkey`),
    isSigner: evalBool(a.signer, ctx, root_overrides, `accounts.${a.name}.signer`),
    isWritable: evalBool(a.writable, ctx, root_overrides, `accounts.${a.name}.writable`),
  }));

  const accountMap = new Map(accounts.map((a) => [a.name, a] as const));
  const get = (...names: string[]) => {
    for (const n of names) {
      const v = accountMap.get(n);
      if (v) return v;
    }
    throw new SolanaCompileError(`Missing required account: ${names[0]}`);
  };

  const data = evalAny(execution.data, ctx, root_overrides, 'data');
  const discriminator = execution.discriminator
    ? evalAny(execution.discriminator, ctx, root_overrides, 'discriminator')
    : undefined;
  const computeUnits = execution.compute_units
    ? evalInt(execution.compute_units, ctx, root_overrides, 'compute_units')
    : undefined;
  const lookupTables = execution.lookup_tables
    ? evalStringArray(execution.lookup_tables, ctx, root_overrides, 'lookup_tables')
    : undefined;

  const ixName = execution.instruction;
  const compileCtx: SolanaInstructionCompilerContext = {
    execution,
    ctx,
    chain: options.chain,
    programId: declaredProgram,
    instruction: ixName,
    accounts,
    accountMap,
    data,
    discriminator,
    getAccount: get,
  };

  const registry = options.compiler_registry ?? getDefaultRegistry();
  const compiler = registry.get(declaredProgram, ixName);
  const tx = compiler ? compiler(compileCtx) : compileGenericInstruction(compileCtx);

  return {
    kind: 'solana_instruction',
    chain: options.chain,
    programId: declaredProgram,
    instruction: ixName,
    tx,
    computeUnits,
    lookupTables,
  };
}

export async function compileSolanaInstructionAsync(
  execution: SolanaInstruction,
  ctx: ResolverContext,
  options: CompileSolanaOptions
): Promise<CompiledSolanaInstructionRequest> {
  const root_overrides = options.params ? { params: options.params } : undefined;

  const declaredProgram = parsePubkey(
    await evalStringAsync(execution.program, ctx, root_overrides, options.detect, 'program'),
    'program'
  );

  const accounts: Array<{
    name: string;
    pubkey: PublicKey;
    isSigner: boolean;
    isWritable: boolean;
  }> = [];
  for (const a of execution.accounts) {
    accounts.push({
      name: a.name,
      pubkey: parsePubkey(
        await evalStringAsync(a.pubkey, ctx, root_overrides, options.detect, `accounts.${a.name}.pubkey`),
        `accounts.${a.name}.pubkey`
      ),
      isSigner: await evalBoolAsync(a.signer, ctx, root_overrides, options.detect, `accounts.${a.name}.signer`),
      isWritable: await evalBoolAsync(a.writable, ctx, root_overrides, options.detect, `accounts.${a.name}.writable`),
    });
  }

  const accountMap = new Map(accounts.map((a) => [a.name, a] as const));
  const get = (...names: string[]) => {
    for (const n of names) {
      const v = accountMap.get(n);
      if (v) return v;
    }
    throw new SolanaCompileError(`Missing required account: ${names[0]}`);
  };

  const data = await evalAnyAsync(execution.data, ctx, root_overrides, options.detect, 'data');
  const discriminator = execution.discriminator
    ? await evalAnyAsync(execution.discriminator, ctx, root_overrides, options.detect, 'discriminator')
    : undefined;
  const computeUnits = execution.compute_units
    ? await evalIntAsync(execution.compute_units, ctx, root_overrides, options.detect, 'compute_units')
    : undefined;
  const lookupTables = execution.lookup_tables
    ? await evalStringArrayAsync(execution.lookup_tables, ctx, root_overrides, options.detect, 'lookup_tables')
    : undefined;

  const ixName = execution.instruction;
  const compileCtx: SolanaInstructionCompilerContext = {
    execution,
    ctx,
    chain: options.chain,
    programId: declaredProgram,
    instruction: ixName,
    accounts,
    accountMap,
    data,
    discriminator,
    getAccount: get,
  };

  const registry = options.compiler_registry ?? getDefaultRegistry();
  const compiler = registry.get(declaredProgram, ixName);
  const tx = compiler ? compiler(compileCtx) : compileGenericInstruction(compileCtx);

  return {
    kind: 'solana_instruction',
    chain: options.chain,
    programId: declaredProgram,
    instruction: ixName,
    tx,
    computeUnits,
    lookupTables,
  };
}

let _defaultRegistry: SolanaInstructionCompilerRegistry | null = null;
function getDefaultRegistry(): SolanaInstructionCompilerRegistry {
  if (_defaultRegistry) return _defaultRegistry;

  const r = new SolanaInstructionCompilerRegistry();

  const registerToken = (programId: PublicKey) => {
    r.register(programId, 'transfer', (c) => {
      const amount = requireBigIntField(c.data, 'amount', 'data.amount');
      const source = c.getAccount('source').pubkey;
      const destination = c.getAccount('destination').pubkey;
      const owner = c.getAccount('authority', 'owner').pubkey;
      return createTransferInstruction(source, destination, owner, amount, [], c.programId);
    });

    r.register(programId, 'transfer_checked', (c) => {
      const amount = requireBigIntField(c.data, 'amount', 'data.amount');
      const decimals = requireIntField(c.data, 'decimals', 'data.decimals');
      const source = c.getAccount('source').pubkey;
      const mint = c.getAccount('mint').pubkey;
      const destination = c.getAccount('destination').pubkey;
      const owner = c.getAccount('authority', 'owner').pubkey;
      return createTransferCheckedInstruction(source, mint, destination, owner, amount, decimals, [], c.programId);
    });

    r.register(programId, 'approve', (c) => {
      const amount = requireBigIntField(c.data, 'amount', 'data.amount');
      const source = c.getAccount('source').pubkey;
      const delegate = c.getAccount('delegate').pubkey;
      const owner = c.getAccount('authority', 'owner').pubkey;
      return createApproveInstruction(source, delegate, owner, amount, [], c.programId);
    });
  };

  registerToken(TOKEN_PROGRAM_ID);
  registerToken(TOKEN_2022_PROGRAM_ID);

  r.register(ASSOCIATED_TOKEN_PROGRAM_ID, 'create_idempotent', (c) => {
    const payer = c.getAccount('payer').pubkey;
    const associatedToken = c.getAccount('associated_token', 'associatedToken').pubkey;
    const owner = c.getAccount('owner').pubkey;
    const mint = c.getAccount('mint').pubkey;
    const tokenProgram =
      c.accountMap.get('token_program')?.pubkey ??
      c.accountMap.get('tokenProgram')?.pubkey ??
      TOKEN_PROGRAM_ID;
    return createAssociatedTokenAccountIdempotentInstruction(
      payer,
      associatedToken,
      owner,
      mint,
      tokenProgram,
      c.programId
    );
  });

  _defaultRegistry = r;
  return r;
}

function compileGenericInstruction(c: SolanaInstructionCompilerContext): TransactionInstruction {
  // Generic: require `data` to resolve to bytes (0x hex or Uint8Array). If `discriminator` is present,
  // prefix it to the bytes.
  const raw = toBytes(c.data, 'data');
  const disc = c.discriminator !== undefined ? toBytes(c.discriminator, 'discriminator') : undefined;
  const finalData = disc ? concatBytes(disc, raw) : raw;

  return new TransactionInstruction({
    programId: c.programId,
    keys: c.accounts.map((a) => ({ pubkey: a.pubkey, isSigner: a.isSigner, isWritable: a.isWritable })),
    data: Buffer.from(finalData),
  });
}

function evalAny(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): unknown {
  try {
    return evaluateValueRef(vref, ctx, root_overrides ? { root_overrides } : undefined);
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      throw new SolanaCompileError(`ValueRef eval failed for ${field}: ${e.message}`, { cause: e });
    }
    throw e;
  }
}

async function evalAnyAsync(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<unknown> {
  try {
    return await evaluateValueRefAsync(vref as any, ctx, root_overrides || detect ? { root_overrides, detect } : undefined);
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      throw new SolanaCompileError(`ValueRef eval failed for ${field}: ${e.message}`, { cause: e });
    }
    throw e;
  }
}

function evalString(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): string {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (typeof v !== 'string') throw new SolanaCompileError(`${field} must resolve to string, got ${typeof v}`);
  return v;
}

async function evalStringAsync(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<string> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (typeof v !== 'string') throw new SolanaCompileError(`${field} must resolve to string, got ${typeof v}`);
  return v;
}

function evalBool(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): boolean {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (typeof v === 'boolean') return v;
  if (typeof v === 'number') {
    if (v === 0) return false;
    if (v === 1) return true;
  }
  if (typeof v === 'string') {
    if (v === 'true') return true;
    if (v === 'false') return false;
  }
  throw new SolanaCompileError(`${field} must resolve to boolean, got ${typeof v}`);
}

async function evalBoolAsync(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<boolean> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (typeof v === 'boolean') return v;
  if (typeof v === 'number') {
    if (v === 0) return false;
    if (v === 1) return true;
  }
  if (typeof v === 'string') {
    if (v === 'true') return true;
    if (v === 'false') return false;
  }
  throw new SolanaCompileError(`${field} must resolve to boolean, got ${typeof v}`);
}

function evalInt(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): number {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) throw new SolanaCompileError(`${field} must be an integer`);
    return v;
  }
  if (typeof v === 'bigint') {
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${field} bigint is too large`);
    return n;
  }
  if (typeof v === 'string') {
    if (!/^\d+$/.test(v)) throw new SolanaCompileError(`${field} must be an integer string`);
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${field} is too large`);
    return n;
  }
  throw new SolanaCompileError(`${field} must resolve to integer, got ${typeof v}`);
}

async function evalIntAsync(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<number> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) throw new SolanaCompileError(`${field} must be an integer`);
    return v;
  }
  if (typeof v === 'bigint') {
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${field} bigint is too large`);
    return n;
  }
  if (typeof v === 'string') {
    if (!/^\d+$/.test(v)) throw new SolanaCompileError(`${field} must be an integer string`);
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${field} is too large`);
    return n;
  }
  throw new SolanaCompileError(`${field} must resolve to integer, got ${typeof v}`);
}

function evalStringArray(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): string[] {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (!Array.isArray(v)) throw new SolanaCompileError(`${field} must resolve to array, got ${typeof v}`);
  for (let i = 0; i < v.length; i++) {
    if (typeof v[i] !== 'string') throw new SolanaCompileError(`${field}[${i}] must be string`);
  }
  return v as string[];
}

async function evalStringArrayAsync(
  vref: SolanaInstruction['program'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<string[]> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (!Array.isArray(v)) throw new SolanaCompileError(`${field} must resolve to array, got ${typeof v}`);
  for (let i = 0; i < v.length; i++) {
    if (typeof (v as any)[i] !== 'string') throw new SolanaCompileError(`${field}[${i}] must be string`);
  }
  return v as string[];
}

function requireBigIntField(obj: unknown, key: string, path: string): bigint {
  if (obj === null || typeof obj !== 'object') throw new SolanaCompileError(`data must resolve to object for this instruction`);
  const v = (obj as any)[key];
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) throw new SolanaCompileError(`${path} must be an integer number`);
    return BigInt(v);
  }
  if (typeof v === 'string') {
    try {
      return BigInt(v);
    } catch {
      throw new SolanaCompileError(`${path} must be bigint-compatible string`);
    }
  }
  throw new SolanaCompileError(`${path} must be bigint/number/string`);
}

function requireIntField(obj: unknown, key: string, path: string): number {
  if (obj === null || typeof obj !== 'object') throw new SolanaCompileError(`data must resolve to object for this instruction`);
  const v = (obj as any)[key];
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) throw new SolanaCompileError(`${path} must be integer`);
    return v;
  }
  if (typeof v === 'bigint') {
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${path} bigint too large`);
    return n;
  }
  if (typeof v === 'string') {
    if (!/^\d+$/.test(v)) throw new SolanaCompileError(`${path} must be an integer string`);
    const n = Number(v);
    if (!Number.isSafeInteger(n)) throw new SolanaCompileError(`${path} too large`);
    return n;
  }
  throw new SolanaCompileError(`${path} must be integer`);
}

function parsePubkey(base58: string, field: string): PublicKey {
  try {
    return new PublicKey(base58);
  } catch (e) {
    throw new SolanaCompileError(`${field} must be a valid base58 pubkey`, { cause: e });
  }
}

function toBytes(value: unknown, field: string): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (typeof value === 'string') {
    const hex = value.startsWith('0x') ? value.slice(2) : value;
    if (!/^[a-fA-F0-9]*$/.test(hex) || hex.length % 2 !== 0) {
      throw new SolanaCompileError(`${field} must be 0x-hex string or Uint8Array`);
    }
    const out = new Uint8Array(hex.length / 2);
    for (let i = 0; i < out.length; i++) {
      out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
  }
  throw new SolanaCompileError(`${field} must be 0x-hex string or Uint8Array`);
}

function concatBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}
