/**
 * EVM compiler (AIS 0.0.2)
 *
 * Compiles `evm_read` / `evm_call` execution specs into concrete requests:
 * - resolves ValueRef (to/args/value)
 * - encodes calldata using JSON ABI (tuple-safe)
 *
 * This module does NOT perform RPC calls nor broadcast transactions.
 */

import { isCoreExecutionSpec, type EvmCall, type EvmRead, type ExecutionSpec, type JsonAbiFunction } from '../../schema/index.js';
import type { ResolverContext } from '../../resolver/index.js';
import { evaluateValueRef, evaluateValueRefAsync, ValueRefEvalError } from '../../resolver/index.js';
import type { DetectResolver } from '../../resolver/value-ref.js';
import { AbiArgsError, AbiEncodingError, encodeJsonAbiFunctionCall } from './encoder.js';

export interface CompileEvmOptions {
  chain: string; // eip155:<n>
  /** Per-node params (shadows runtime.params during compilation) */
  params?: Record<string, unknown>;
  /** Optional detect resolver (may be async) */
  detect?: DetectResolver;
}

export interface CompiledEvmAbiRequest {
  kind: 'evm_call' | 'evm_read';
  chain: string;
  chainId: number;
  to: string;
  data: string;
  value: bigint;
  abi: JsonAbiFunction;
  args?: Record<string, unknown>;
}

export interface CompiledEvmRpcRequest {
  kind: 'evm_rpc';
  chain: string;
  chainId: number;
  method: string;
  params: unknown[];
}

export type CompiledEvmRequest = CompiledEvmAbiRequest | CompiledEvmRpcRequest;

type EvmRpcExecution = {
  type: 'evm_rpc';
  method: string;
  params?: unknown;
};

export class EvmCompileError extends Error {
  constructor(message: string, public readonly details?: unknown) {
    super(message);
    this.name = 'EvmCompileError';
  }
}

export function compileEvmExecution(
  execution: ExecutionSpec,
  ctx: ResolverContext,
  options: CompileEvmOptions
): CompiledEvmRequest {
  if (!isCoreExecutionSpec(execution)) {
    const t = (execution as any)?.type;
    if (t === 'evm_rpc') return compileEvmRpc(execution as any as EvmRpcExecution, ctx, options);
    throw new EvmCompileError(`Unsupported execution type for EVM compiler: ${t}`);
  }
  if (execution.type === 'evm_call') return compileEvmCall(execution, ctx, options);
  if (execution.type === 'evm_read') return compileEvmRead(execution, ctx, options);
  throw new EvmCompileError(`Unsupported execution type for EVM compiler: ${execution.type}`);
}

export async function compileEvmExecutionAsync(
  execution: ExecutionSpec,
  ctx: ResolverContext,
  options: CompileEvmOptions
): Promise<CompiledEvmRequest> {
  if (!isCoreExecutionSpec(execution)) {
    const t = (execution as any)?.type;
    if (t === 'evm_rpc') return await compileEvmRpcAsync(execution as any as EvmRpcExecution, ctx, options);
    throw new EvmCompileError(`Unsupported execution type for EVM compiler: ${t}`);
  }
  if (execution.type === 'evm_call') return await compileEvmCallAsync(execution, ctx, options);
  if (execution.type === 'evm_read') return await compileEvmReadAsync(execution, ctx, options);
  throw new EvmCompileError(`Unsupported execution type for EVM compiler: ${execution.type}`);
}

export function compileEvmCall(
  execution: EvmCall,
  ctx: ResolverContext,
  options: CompileEvmOptions
): CompiledEvmRequest {
  const { to, args, value } = resolveEvmInputs(execution.to, execution.args, execution.value, ctx, options);
  const data = encodeWithErrors(execution.abi, args);
  return {
    kind: 'evm_call',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    to,
    data,
    value,
    abi: execution.abi,
    args,
  };
}

export async function compileEvmCallAsync(
  execution: EvmCall,
  ctx: ResolverContext,
  options: CompileEvmOptions
): Promise<CompiledEvmRequest> {
  const { to, args, value } = await resolveEvmInputsAsync(execution.to, execution.args, execution.value, ctx, options);
  const data = encodeWithErrors(execution.abi, args);
  return {
    kind: 'evm_call',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    to,
    data,
    value,
    abi: execution.abi,
    args,
  };
}

export function compileEvmRead(
  execution: EvmRead,
  ctx: ResolverContext,
  options: CompileEvmOptions
): CompiledEvmRequest {
  const { to, args } = resolveEvmInputs(execution.to, execution.args, undefined, ctx, options);
  const data = encodeWithErrors(execution.abi, args);
  return {
    kind: 'evm_read',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    to,
    data,
    value: 0n,
    abi: execution.abi,
    args,
  };
}

export async function compileEvmReadAsync(
  execution: EvmRead,
  ctx: ResolverContext,
  options: CompileEvmOptions
): Promise<CompiledEvmRequest> {
  const { to, args } = await resolveEvmInputsAsync(execution.to, execution.args, undefined, ctx, options);
  const data = encodeWithErrors(execution.abi, args);
  return {
    kind: 'evm_read',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    to,
    data,
    value: 0n,
    abi: execution.abi,
    args,
  };
}

function compileEvmRpc(
  execution: EvmRpcExecution,
  ctx: ResolverContext,
  options: CompileEvmOptions
): CompiledEvmRequest {
  const method = String(execution?.method ?? '').trim();
  if (!method) throw new EvmCompileError('evm_rpc.method must be a non-empty string');

  const root_overrides = options.params ? { params: options.params } : undefined;
  const paramsValue = execution.params !== undefined ? evalAny(execution.params as any, ctx, root_overrides, 'params') : [];
  if (!Array.isArray(paramsValue)) {
    throw new EvmCompileError(`evm_rpc.params must resolve to an array, got ${typeof paramsValue}`);
  }

  // Common ergonomic normalization: accept number/bigint for block tag in eth_getBalance.
  if (method === 'eth_getBalance') {
    if (paramsValue.length < 2 || paramsValue[1] === undefined || paramsValue[1] === null) {
      paramsValue[1] = 'latest';
    } else {
      paramsValue[1] = formatBlockTag(paramsValue[1]);
    }
  }

  return {
    kind: 'evm_rpc',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    method,
    params: paramsValue,
  };
}

async function compileEvmRpcAsync(
  execution: EvmRpcExecution,
  ctx: ResolverContext,
  options: CompileEvmOptions
): Promise<CompiledEvmRequest> {
  const method = String(execution?.method ?? '').trim();
  if (!method) throw new EvmCompileError('evm_rpc.method must be a non-empty string');

  const root_overrides = options.params ? { params: options.params } : undefined;
  const paramsValue =
    execution.params !== undefined
      ? await evalAnyAsync(execution.params as any, ctx, root_overrides, options.detect, 'params')
      : [];
  if (!Array.isArray(paramsValue)) {
    throw new EvmCompileError(`evm_rpc.params must resolve to an array, got ${typeof paramsValue}`);
  }

  if (method === 'eth_getBalance') {
    if (paramsValue.length < 2 || paramsValue[1] === undefined || paramsValue[1] === null) {
      paramsValue[1] = 'latest';
    } else {
      paramsValue[1] = formatBlockTag(paramsValue[1]);
    }
  }

  return {
    kind: 'evm_rpc',
    chain: options.chain,
    chainId: parseEip155ChainId(options.chain),
    method,
    params: paramsValue,
  };
}

function resolveEvmInputs(
  toRef: EvmCall['to'],
  argsRef: EvmCall['args'],
  valueRef: EvmCall['value'] | undefined,
  ctx: ResolverContext,
  options: CompileEvmOptions
): { to: string; args: Record<string, unknown>; value: bigint } {
  const root_overrides = options.params ? { params: options.params } : undefined;

  const to = evalString(toRef, ctx, root_overrides, 'to');

  const args: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(argsRef)) {
    args[k] = evalAny(v, ctx, root_overrides, `args.${k}`);
  }

  const value = valueRef ? evalBigInt(valueRef, ctx, root_overrides, 'value') : 0n;

  return { to, args, value };
}

async function resolveEvmInputsAsync(
  toRef: EvmCall['to'],
  argsRef: EvmCall['args'],
  valueRef: EvmCall['value'] | undefined,
  ctx: ResolverContext,
  options: CompileEvmOptions
): Promise<{ to: string; args: Record<string, unknown>; value: bigint }> {
  const root_overrides = options.params ? { params: options.params } : undefined;

  const to = await evalStringAsync(toRef, ctx, root_overrides, options.detect, 'to');

  const args: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(argsRef)) {
    args[k] = await evalAnyAsync(v, ctx, root_overrides, options.detect, `args.${k}`);
  }

  const value = valueRef ? await evalBigIntAsync(valueRef, ctx, root_overrides, options.detect, 'value') : 0n;
  return { to, args, value };
}

function encodeWithErrors(abi: JsonAbiFunction, args: Record<string, unknown>): string {
  try {
    return encodeJsonAbiFunctionCall(abi, args);
  } catch (e) {
    if (e instanceof AbiArgsError || e instanceof AbiEncodingError) {
      throw new EvmCompileError(e.message, { cause: e });
    }
    throw e;
  }
}

function evalAny(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): unknown {
  try {
    return evaluateValueRef(vref, ctx, root_overrides ? { root_overrides } : undefined);
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      throw new EvmCompileError(`ValueRef eval failed for ${field}: ${e.message}`, { cause: e });
    }
    throw e;
  }
}

async function evalAnyAsync(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<unknown> {
  try {
    return await evaluateValueRefAsync(vref as any, ctx, root_overrides || detect ? { root_overrides, detect } : undefined);
  } catch (e) {
    if (e instanceof ValueRefEvalError) {
      throw new EvmCompileError(`ValueRef eval failed for ${field}: ${e.message}`, { cause: e });
    }
    throw e;
  }
}

function evalString(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): string {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (typeof v !== 'string') {
    throw new EvmCompileError(`${field} must resolve to string, got ${typeof v}`);
  }
  if (!/^0x[a-fA-F0-9]{40}$/.test(v)) {
    throw new EvmCompileError(`${field} must be a 0x-prefixed 20-byte hex address`);
  }
  return v;
}

async function evalStringAsync(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<string> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (typeof v !== 'string') {
    throw new EvmCompileError(`${field} must resolve to string, got ${typeof v}`);
  }
  if (!/^0x[a-fA-F0-9]{40}$/.test(v)) {
    throw new EvmCompileError(`${field} must be a 0x-prefixed 20-byte hex address`);
  }
  return v;
}

function evalBigInt(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  field: string
): bigint {
  const v = evalAny(vref, ctx, root_overrides, field);
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) {
      throw new EvmCompileError(`${field} must be an integer number`);
    }
    return BigInt(v);
  }
  if (typeof v === 'string') {
    try {
      return BigInt(v);
    } catch {
      throw new EvmCompileError(`${field} must be a bigint-compatible string`);
    }
  }
  throw new EvmCompileError(`${field} must resolve to bigint/number/string, got ${typeof v}`);
}

async function evalBigIntAsync(
  vref: EvmCall['to'],
  ctx: ResolverContext,
  root_overrides: Record<string, unknown> | undefined,
  detect: DetectResolver | undefined,
  field: string
): Promise<bigint> {
  const v = await evalAnyAsync(vref, ctx, root_overrides, detect, field);
  if (typeof v === 'bigint') return v;
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v)) {
      throw new EvmCompileError(`${field} must be an integer number`);
    }
    return BigInt(v);
  }
  if (typeof v === 'string') {
    try {
      return BigInt(v);
    } catch {
      throw new EvmCompileError(`${field} must be a bigint-compatible string`);
    }
  }
  throw new EvmCompileError(`${field} must resolve to bigint/number/string, got ${typeof v}`);
}

function parseEip155ChainId(chain: string): number {
  const m = chain.match(/^eip155:(\d+)$/);
  if (!m) throw new EvmCompileError(`Invalid chain for EVM compilation: ${chain}`);
  const n = Number(m[1]);
  if (!Number.isSafeInteger(n) || n <= 0) throw new EvmCompileError(`Invalid eip155 chain id: ${chain}`);
  return n;
}

function formatBlockTag(v: unknown): string {
  if (v === undefined || v === null) return 'latest';
  if (typeof v === 'string') {
    const s = v.trim();
    return s === '' ? 'latest' : s;
  }
  if (typeof v === 'bigint') return toHexQuantity(v);
  if (typeof v === 'number') {
    if (!Number.isFinite(v) || !Number.isInteger(v) || v < 0) {
      throw new EvmCompileError('block_tag must be a non-negative integer');
    }
    return toHexQuantity(BigInt(v));
  }
  throw new EvmCompileError(`block_tag must resolve to string/number/bigint, got ${typeof v}`);
}

function toHexQuantity(n: bigint): string {
  if (n < 0n) throw new EvmCompileError('block_tag must be non-negative');
  return `0x${n.toString(16)}`;
}
