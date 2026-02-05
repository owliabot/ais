/**
 * EVM Multicall Builder
 * 
 * Supports batched write transactions in a single atomic call:
 * - Standard multicall: multicall(bytes[]) on same contract
 * - Multicall3: aggregate3 for cross-contract batching
 * - Universal Router: execute(commands, inputs, deadline) for Uniswap
 */

import type { EvmMulticall } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import type { CELContext } from '../cel/evaluator.js';
import type { Evaluator } from '../cel/evaluator.js';
import type { ProtocolSpec } from '../schema/index.js';
import type { TransactionRequest } from './builder.js';
import { encodeFunctionCall, buildFunctionSignature, encodeValue } from './encoder.js';
import { getContractAddress } from '../resolver/reference.js';
import { keccak256 } from './keccak.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

export interface MulticallBuildOptions {
  /** Chain identifier (e.g., "eip155:1") */
  chain: string;
  /** Multicall encoding style */
  style?: 'standard' | 'multicall3' | 'universal_router';
}

export interface EncodedCall {
  /** Target contract (for multicall3) */
  target?: string;
  /** Encoded calldata */
  data: string;
  /** Allow failure (for multicall3) */
  allowFailure?: boolean;
  /** Command byte (for universal router) */
  command?: number;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Parse chain ID from chain string
 */
function parseChainId(chain: string): number {
  const match = chain.match(/^eip155:(\d+)$/);
  if (!match) {
    throw new Error(`Unsupported chain format: ${chain}`);
  }
  return parseInt(match[1], 10);
}

/**
 * Resolve a mapping value to its final form
 */
function resolveMappingValue(
  value: unknown,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string
): unknown {
  if (typeof value !== 'string') {
    if (typeof value === 'number') {
      return BigInt(value);
    }
    return value;
  }

  // Literal values
  if (value.startsWith('0x')) return value;
  if (/^-?\d+$/.test(value)) return BigInt(value);
  if (value === 'true') return true;
  if (value === 'false') return false;

  // CEL expression check
  if (/\w+\s*\(/.test(value) || /[+\-*/%<>=!&|?:]/.test(value)) {
    const result = evaluator.evaluate(value, celCtx);
    if (typeof result === 'number') {
      return BigInt(Math.floor(result));
    }
    return result;
  }

  // Reference patterns
  const parts = value.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'params': {
      const key = parts.slice(1).join('.');
      return ctx.variables[`params.${key}`] ?? ctx.variables[key];
    }
    case 'calculated': {
      const key = parts.slice(1).join('.');
      return ctx.variables[`calculated.${key}`];
    }
    case 'ctx': {
      const key = parts.slice(1).join('.');
      return ctx.variables[`ctx.${key}`];
    }
    case 'contracts': {
      const contractName = parts[1];
      const addr = getContractAddress(protocol, chain, contractName);
      if (!addr) {
        throw new Error(`Contract "${contractName}" not found for chain "${chain}"`);
      }
      return addr;
    }
    default:
      return value;
  }
}

/**
 * Evaluate a condition expression
 */
function evaluateCondition(
  condition: string | undefined,
  celCtx: CELContext,
  evaluator: Evaluator
): boolean {
  if (!condition) return true;
  try {
    return Boolean(evaluator.evaluate(condition.trim(), celCtx));
  } catch {
    return false;
  }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Call Encoding
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Encode a single call's calldata
 */
function encodeCallData(
  func: string,
  abi: string,
  mapping: Record<string, unknown>,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string
): string {
  // Parse ABI
  const abiMatch = abi.match(/^\((.*)\)$/);
  if (!abiMatch) {
    throw new Error(`Invalid ABI format: ${abi}`);
  }

  const types = abiMatch[1]
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);

  // Resolve mapping values
  const values: unknown[] = [];
  for (const mappingValue of Object.values(mapping)) {
    const resolved = resolveMappingValue(mappingValue, ctx, celCtx, evaluator, protocol, chain);
    values.push(resolved);
  }

  const signature = buildFunctionSignature(func, types);
  return encodeFunctionCall(signature, types, values);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Multicall Encoding Styles
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Encode as standard multicall: multicall(bytes[])
 * All calls are to the same contract (the multicall contract itself)
 */
function encodeStandardMulticall(calls: EncodedCall[]): string {
  // multicall(bytes[] calldata data)
  const selector = keccak256('multicall(bytes[])').slice(0, 10);
  
  // Encode bytes[] array
  const encodedArray = encodeBytesDynamicArray(calls.map((c) => c.data));
  
  return selector + encodedArray.slice(2);
}

/**
 * Encode as Multicall3 aggregate3: aggregate3((address,bool,bytes)[])
 * Allows calling different contracts
 */
function encodeMulticall3(calls: EncodedCall[]): string {
  // aggregate3((address target, bool allowFailure, bytes callData)[] calls)
  const selector = keccak256('aggregate3((address,bool,bytes)[])').slice(0, 10);
  
  // Build the tuple array
  const tuples = calls.map((c) => ({
    target: c.target ?? '0x0000000000000000000000000000000000000000',
    allowFailure: c.allowFailure ?? false,
    callData: c.data,
  }));
  
  const encodedArray = encodeTupleArray(tuples);
  
  return selector + encodedArray.slice(2);
}

/**
 * Encode as Universal Router execute: execute(bytes,bytes[],uint256)
 * For Uniswap Universal Router pattern
 */
function encodeUniversalRouter(calls: EncodedCall[], deadline: bigint): string {
  // execute(bytes commands, bytes[] inputs, uint256 deadline)
  const selector = keccak256('execute(bytes,bytes[],uint256)').slice(0, 10);
  
  // Commands are single bytes indicating the operation
  const commands = calls.map((c) => c.command ?? 0);
  const commandsHex = '0x' + commands.map((c) => c.toString(16).padStart(2, '0')).join('');
  
  // Inputs are the encoded call data for each command
  const inputs = calls.map((c) => c.data);
  
  // Encode: offset to commands, offset to inputs, deadline, commands data, inputs data
  const encoded = encodeExecuteParams(commandsHex, inputs, deadline);
  
  return selector + encoded.slice(2);
}

// ═══════════════════════════════════════════════════════════════════════════════
// ABI Encoding Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Encode a dynamic bytes array
 */
function encodeBytesDynamicArray(items: string[]): string {
  // Offset to array data (32 bytes)
  let result = '0x' + (32n).toString(16).padStart(64, '0');
  
  // Array length
  result += BigInt(items.length).toString(16).padStart(64, '0');
  
  // Calculate offsets for each bytes element
  // First offset starts after all offset slots
  let currentOffset = BigInt(items.length * 32);
  const offsets: bigint[] = [];
  const encodedItems: string[] = [];
  
  for (const item of items) {
    offsets.push(currentOffset);
    // Remove 0x prefix if present
    const data = item.startsWith('0x') ? item.slice(2) : item;
    // Bytes length + padded data
    const paddedLength = Math.ceil(data.length / 64) * 64;
    const encoded = BigInt(data.length / 2).toString(16).padStart(64, '0') + 
                   data.padEnd(paddedLength, '0');
    encodedItems.push(encoded);
    currentOffset += BigInt(32 + paddedLength / 2);
  }
  
  // Add offsets
  for (const offset of offsets) {
    result += offset.toString(16).padStart(64, '0');
  }
  
  // Add encoded items
  for (const item of encodedItems) {
    result += item;
  }
  
  return result;
}

/**
 * Encode tuple array for Multicall3
 */
function encodeTupleArray(tuples: Array<{ target: string; allowFailure: boolean; callData: string }>): string {
  // Offset to array data
  let result = '0x' + (32n).toString(16).padStart(64, '0');
  
  // Array length
  result += BigInt(tuples.length).toString(16).padStart(64, '0');
  
  // Each tuple is: address (32) + bool (32) + offset to bytes (32) + bytes data
  // Calculate total static size per tuple = 3 * 32 = 96 bytes
  const staticSize = 96n;
  
  // First, calculate offsets to each tuple's dynamic data (callData)
  let dynamicOffset = BigInt(tuples.length) * staticSize;
  const tupleData: string[] = [];
  
  for (const tuple of tuples) {
    // Encode static parts
    const target = tuple.target.slice(2).padStart(64, '0');
    const allowFailure = (tuple.allowFailure ? 1n : 0n).toString(16).padStart(64, '0');
    
    // Offset to callData within this tuple (relative)
    const callDataOffset = (64n).toString(16).padStart(64, '0'); // 96 - 32 = 64
    
    tupleData.push(target + allowFailure + callDataOffset);
    
    // Encode callData
    const data = tuple.callData.startsWith('0x') ? tuple.callData.slice(2) : tuple.callData;
    const paddedLength = Math.ceil(data.length / 64) * 64;
    const encodedCallData = BigInt(data.length / 2).toString(16).padStart(64, '0') + 
                           data.padEnd(paddedLength, '0');
    tupleData.push(encodedCallData);
  }
  
  // Add all tuple data
  result += tupleData.join('');
  
  return result;
}

/**
 * Encode Universal Router execute params
 */
function encodeExecuteParams(commands: string, inputs: string[], deadline: bigint): string {
  // Dynamic encoding: offset_commands, offset_inputs, deadline, commands_data, inputs_data
  
  // Static part: 3 * 32 bytes (two offsets + deadline)
  const staticSize = 96n;
  
  // Commands offset (after static part)
  const commandsOffset = staticSize;
  
  // Commands data
  const commandsData = commands.startsWith('0x') ? commands.slice(2) : commands;
  const commandsLen = BigInt(commandsData.length / 2);
  const commandsPadded = Math.ceil(commandsData.length / 64) * 64;
  const encodedCommands = commandsLen.toString(16).padStart(64, '0') + 
                         commandsData.padEnd(commandsPadded, '0');
  
  // Inputs offset (after commands)
  const inputsOffset = commandsOffset + BigInt(32 + commandsPadded / 2);
  
  // Encode inputs as bytes[]
  const inputsArrayLen = BigInt(inputs.length);
  let inputsData = inputsArrayLen.toString(16).padStart(64, '0');
  
  // Calculate offsets for each input
  let currentOffset = BigInt(inputs.length * 32);
  const inputOffsets: bigint[] = [];
  const encodedInputs: string[] = [];
  
  for (const input of inputs) {
    inputOffsets.push(currentOffset);
    const data = input.startsWith('0x') ? input.slice(2) : input;
    const paddedLength = Math.ceil(data.length / 64) * 64;
    const encoded = BigInt(data.length / 2).toString(16).padStart(64, '0') + 
                   data.padEnd(paddedLength, '0');
    encodedInputs.push(encoded);
    currentOffset += BigInt(32 + paddedLength / 2);
  }
  
  for (const offset of inputOffsets) {
    inputsData += offset.toString(16).padStart(64, '0');
  }
  inputsData += encodedInputs.join('');
  
  // Build result
  let result = '0x';
  result += commandsOffset.toString(16).padStart(64, '0');
  result += inputsOffset.toString(16).padStart(64, '0');
  result += deadline.toString(16).padStart(64, '0');
  result += encodedCommands;
  result += inputsData;
  
  return result;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Main Builder
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Build an evm_multicall transaction
 */
export function buildEvmMulticall(
  protocol: ProtocolSpec,
  spec: EvmMulticall,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  chain: string,
  options: MulticallBuildOptions = { chain }
): TransactionRequest {
  const chainId = parseChainId(chain);

  // Resolve contract address
  let contractAddress: string;
  if (spec.contract.startsWith('0x')) {
    contractAddress = spec.contract;
  } else {
    const addr = getContractAddress(protocol, chain, spec.contract);
    if (!addr) {
      throw new Error(`Contract "${spec.contract}" not found for chain "${chain}"`);
    }
    contractAddress = addr;
  }

  // Build encoded calls, filtering by conditions
  const encodedCalls: EncodedCall[] = [];

  for (const call of spec.calls) {
    // Evaluate condition
    if (!evaluateCondition(call.condition, celCtx, evaluator)) {
      continue; // Skip this call
    }

    const callData = encodeCallData(
      call.function,
      call.abi,
      call.mapping as Record<string, unknown>,
      ctx,
      celCtx,
      evaluator,
      protocol,
      chain
    );

    encodedCalls.push({
      data: callData,
      target: contractAddress, // Same contract for standard multicall
      allowFailure: false,
    });
  }

  if (encodedCalls.length === 0) {
    throw new Error('No calls to execute after condition evaluation');
  }

  // Determine encoding style
  const style = options.style ?? 'standard';

  // Resolve deadline if specified
  let deadline = 0n;
  if (spec.deadline) {
    const resolved = resolveMappingValue(
      spec.deadline,
      ctx,
      celCtx,
      evaluator,
      protocol,
      chain
    );
    if (typeof resolved === 'bigint') {
      deadline = resolved;
    } else if (typeof resolved === 'number') {
      deadline = BigInt(resolved);
    } else if (typeof resolved === 'string' && /^\d+$/.test(resolved)) {
      deadline = BigInt(resolved);
    }
  }

  // Encode based on style
  let data: string;
  switch (style) {
    case 'multicall3':
      data = encodeMulticall3(encodedCalls);
      break;
    case 'universal_router':
      data = encodeUniversalRouter(encodedCalls, deadline);
      break;
    case 'standard':
    default:
      data = encodeStandardMulticall(encodedCalls);
      break;
  }

  return {
    to: contractAddress,
    data,
    value: 0n,
    chainId,
    stepId: 'multicall',
    stepDescription: `Batched ${encodedCalls.length} calls`,
  };
}

/**
 * Build individual call data for inspection/testing
 */
export function buildMulticallCalls(
  protocol: ProtocolSpec,
  spec: EvmMulticall,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  chain: string
): EncodedCall[] {
  const calls: EncodedCall[] = [];

  for (const call of spec.calls) {
    if (!evaluateCondition(call.condition, celCtx, evaluator)) {
      continue;
    }

    const callData = encodeCallData(
      call.function,
      call.abi,
      call.mapping as Record<string, unknown>,
      ctx,
      celCtx,
      evaluator,
      protocol,
      chain
    );

    calls.push({ data: callData });
  }

  return calls;
}

// Export encoding functions for advanced use
export {
  encodeStandardMulticall,
  encodeMulticall3,
  encodeUniversalRouter,
};
