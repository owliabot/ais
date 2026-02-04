/**
 * Execution Builder - build transaction calldata from AIS actions
 */

import type { Action, ProtocolSpec, Param } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { getContractAddress } from '../resolver/reference.js';
import { resolveExpressionString, hasExpressions } from '../resolver/expression.js';
import { encodeFunctionCall, buildFunctionSignature } from './encoder.js';

export interface TransactionRequest {
  /** Target contract address */
  to: string;
  /** Encoded calldata */
  data: string;
  /** ETH value to send (in wei) */
  value: bigint;
  /** Chain ID */
  chainId: number;
}

export interface BuildOptions {
  /** Chain to execute on (e.g., "eip155:1") */
  chain: string;
  /** Override contract address */
  contractAddress?: string;
  /** ETH value to send */
  value?: bigint;
}

export interface BuildResult {
  success: true;
  transaction: TransactionRequest;
  action: Action;
  resolvedParams: Record<string, unknown>;
}

export interface BuildError {
  success: false;
  error: string;
  details?: unknown;
}

export type BuildOutput = BuildResult | BuildError;

/**
 * Map AIS types to Solidity types
 */
function mapToSolidityType(aisType: string): string {
  const typeMap: Record<string, string> = {
    address: 'address',
    bool: 'bool',
    string: 'string',
    bytes: 'bytes',
    asset: 'address', // asset resolves to token address
    token_amount: 'uint256', // token_amount resolves to raw amount
  };

  // Direct match
  if (typeMap[aisType]) {
    return typeMap[aisType];
  }

  // uint/int types
  if (/^u?int\d*$/.test(aisType)) {
    return aisType.includes('int') && !aisType.startsWith('uint')
      ? aisType
      : aisType.replace('uint', 'uint');
  }

  // bytes1-32
  if (/^bytes\d+$/.test(aisType)) {
    return aisType;
  }

  // Default to the type as-is
  return aisType;
}

/**
 * Resolve parameter value, handling expressions and type coercion
 */
function resolveParamValue(
  param: Param,
  inputValue: unknown,
  ctx: ResolverContext
): unknown {
  let value = inputValue;

  // Resolve expressions in string values
  if (typeof value === 'string' && hasExpressions(value)) {
    value = resolveExpressionString(value, ctx);
  }

  // Use default if no value provided
  if (value === undefined && param.default !== undefined) {
    value = param.default;
  }

  // Type coercion
  const solType = mapToSolidityType(param.type);

  if (solType === 'address') {
    if (typeof value !== 'string' || !value.startsWith('0x')) {
      throw new Error(`Invalid address for ${param.name}: ${value}`);
    }
    return value;
  }

  if (solType.startsWith('uint') || solType.startsWith('int')) {
    if (typeof value === 'string') {
      return BigInt(value);
    }
    if (typeof value === 'number') {
      return BigInt(value);
    }
    if (typeof value === 'bigint') {
      return value;
    }
    throw new Error(`Invalid integer for ${param.name}: ${value}`);
  }

  if (solType === 'bool') {
    return Boolean(value);
  }

  return value;
}

/**
 * Parse chain ID from chain string (e.g., "eip155:1" → 1)
 */
function parseChainId(chain: string): number {
  const match = chain.match(/^eip155:(\d+)$/);
  if (!match) {
    throw new Error(`Invalid chain format: ${chain}. Expected "eip155:<chainId>"`);
  }
  return parseInt(match[1], 10);
}

/**
 * Build a transaction request from an AIS action
 */
export function buildTransaction(
  protocol: ProtocolSpec,
  action: Action,
  inputs: Record<string, unknown>,
  ctx: ResolverContext,
  options: BuildOptions
): BuildOutput {
  try {
    const { chain, contractAddress, value = 0n } = options;

    // Get contract address
    let to: string;
    if (contractAddress) {
      to = contractAddress;
    } else {
      const addr = getContractAddress(protocol, chain, action.contract);
      if (!addr) {
        return {
          success: false,
          error: `Contract "${action.contract}" not found for chain "${chain}"`,
        };
      }
      to = addr;
    }

    // Get parameter types and values
    const params = action.params ?? [];
    const types: string[] = [];
    const values: unknown[] = [];
    const resolvedParams: Record<string, unknown> = {};

    for (const param of params) {
      const inputValue = inputs[param.name];

      // Check required params
      if (inputValue === undefined && param.required !== false && param.default === undefined) {
        return {
          success: false,
          error: `Missing required parameter: ${param.name}`,
        };
      }

      try {
        const resolved = resolveParamValue(param, inputValue, ctx);
        const solType = mapToSolidityType(param.type);
        types.push(solType);
        values.push(resolved);
        resolvedParams[param.name] = resolved;
      } catch (err) {
        return {
          success: false,
          error: `Failed to resolve parameter ${param.name}: ${err instanceof Error ? err.message : err}`,
        };
      }
    }

    // Build function signature and encode call
    const signature = buildFunctionSignature(action.method, types);
    const data = encodeFunctionCall(signature, types, values);

    return {
      success: true,
      transaction: {
        to,
        data,
        value,
        chainId: parseChainId(chain),
      },
      action,
      resolvedParams,
    };
  } catch (err) {
    return {
      success: false,
      error: err instanceof Error ? err.message : String(err),
      details: err,
    };
  }
}

/**
 * Build a query call (for eth_call)
 */
export function buildQuery(
  protocol: ProtocolSpec,
  query: { contract: string; method: string; params?: Param[] },
  inputs: Record<string, unknown>,
  ctx: ResolverContext,
  options: BuildOptions
): BuildOutput {
  // Queries use the same encoding as transactions
  const pseudoAction: Action = {
    contract: query.contract,
    method: query.method,
    params: query.params,
  };

  const result = buildTransaction(protocol, pseudoAction, inputs, ctx, {
    ...options,
    value: 0n, // Queries never send value
  });

  return result;
}

/**
 * Build multiple transactions for a workflow
 */
export function buildWorkflowTransactions(
  protocols: Map<string, ProtocolSpec>,
  nodes: Array<{
    skill: string;
    action?: string;
    query?: string;
    args?: Record<string, unknown>;
  }>,
  ctx: ResolverContext,
  chain: string
): Array<BuildOutput & { nodeIndex: number }> {
  const results: Array<BuildOutput & { nodeIndex: number }> = [];

  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i];
    const [protocolName] = node.skill.split('@');
    const protocol = protocols.get(protocolName);

    if (!protocol) {
      results.push({
        nodeIndex: i,
        success: false,
        error: `Protocol not found: ${protocolName}`,
      });
      continue;
    }

    const inputs = node.args ?? {};

    if (node.action) {
      const action = protocol.actions[node.action];
      if (!action) {
        results.push({
          nodeIndex: i,
          success: false,
          error: `Action not found: ${node.action}`,
        });
        continue;
      }
      results.push({
        nodeIndex: i,
        ...buildTransaction(protocol, action, inputs, ctx, { chain }),
      });
    } else if (node.query && protocol.queries) {
      const query = protocol.queries[node.query];
      if (!query) {
        results.push({
          nodeIndex: i,
          success: false,
          error: `Query not found: ${node.query}`,
        });
        continue;
      }
      results.push({
        nodeIndex: i,
        ...buildQuery(protocol, query, inputs, ctx, { chain }),
      });
    }
  }

  return results;
}
