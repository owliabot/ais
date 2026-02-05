/**
 * Execution Builder - build transaction calldata from AIS actions
 * 
 * NOTE: This module is being refactored to support the new execution block structure.
 * Full implementation pending.
 */

import type {
  Action,
  Query,
  ProtocolSpec,
  Param,
  ExecutionSpec,
  EvmCall,
  EvmRead,
  Composite,
  CompositeStep,
} from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { getContractAddress } from '../resolver/reference.js';
import { resolveExpressionString, hasExpressions } from '../resolver/expression.js';
import { encodeFunctionCall, buildFunctionSignature } from './encoder.js';

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

export interface TransactionRequest {
  /** Target contract address */
  to: string;
  /** Encoded calldata */
  data: string;
  /** ETH value to send (in wei) */
  value: bigint;
  /** Chain ID */
  chainId: number;
  /** Step ID (for composite execution) */
  stepId?: string;
  /** Step description */
  stepDescription?: string;
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
  transactions: TransactionRequest[];
  action: Action;
  resolvedParams: Record<string, unknown>;
}

export interface BuildError {
  success: false;
  error: string;
  details?: unknown;
}

export type BuildOutput = BuildResult | BuildError;

// ═══════════════════════════════════════════════════════════════════════════════
// Chain Pattern Matching
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Match chain against execution patterns
 * Returns the most specific matching ExecutionSpec
 */
export function matchChainPattern(
  chain: string,
  execution: Record<string, ExecutionSpec>
): ExecutionSpec | null {
  // Exact match first
  if (execution[chain]) {
    return execution[chain];
  }

  // Pattern match (e.g., "eip155:*" matches "eip155:1")
  const [namespace] = chain.split(':');
  const wildcardPattern = `${namespace}:*`;
  if (execution[wildcardPattern]) {
    return execution[wildcardPattern];
  }

  // Global fallback
  if (execution['*']) {
    return execution['*'];
  }

  return null;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Type Mapping
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Map AIS types to Solidity types
 */
function mapToSolidityType(aisType: string): string {
  const typeMap: Record<string, string> = {
    address: 'address',
    bool: 'bool',
    string: 'string',
    bytes: 'bytes',
    asset: 'address',
    token_amount: 'uint256',
    float: 'uint256', // Floats are typically converted to atomic amounts
  };

  if (typeMap[aisType]) {
    return typeMap[aisType];
  }

  // uint/int types
  if (/^u?int\d*$/.test(aisType)) {
    return aisType;
  }

  // bytes1-32
  if (/^bytes\d+$/.test(aisType)) {
    return aisType;
  }

  return aisType;
}

/**
 * Parse chain ID from chain string (e.g., "eip155:1" → 1)
 */
function parseChainId(chain: string): number {
  const match = chain.match(/^eip155:(\d+)$/);
  if (!match) {
    throw new Error(`Unsupported chain format: ${chain}. Only EVM chains supported.`);
  }
  return parseInt(match[1], 10);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Parameter Resolution
// ═══════════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════════
// Transaction Building
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Build a single EVM call transaction
 */
function buildEvmCall(
  protocol: ProtocolSpec,
  spec: EvmCall | EvmRead,
  ctx: ResolverContext,
  chain: string,
  stepId?: string,
  stepDescription?: string
): TransactionRequest {
  // Resolve contract address
  let to: string;
  const contractRef = spec.contract;
  
  if (contractRef.startsWith('0x')) {
    // Direct address
    to = contractRef;
  } else if (contractRef.startsWith('params.')) {
    // Reference to param (e.g., "params.token_in.address")
    // TODO: Resolve from context
    throw new Error('Param references in contract not yet implemented');
  } else {
    // Contract name from deployments
    const addr = getContractAddress(protocol, chain, contractRef);
    if (!addr) {
      throw new Error(`Contract "${contractRef}" not found for chain "${chain}"`);
    }
    to = addr;
  }

  // Parse ABI to get types
  // ABI format: "(type1,type2,...)" or "((type1,type2))" for structs
  const abiMatch = spec.abi.match(/^\((.*)\)$/);
  if (!abiMatch) {
    throw new Error(`Invalid ABI format: ${spec.abi}`);
  }
  
  // Split types (simple split for now, doesn't handle nested tuples)
  const types = abiMatch[1]
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);

  // Resolve mapping values
  const values: unknown[] = [];
  const mappingEntries = Object.entries(spec.mapping);
  
  for (const [, mappingValue] of mappingEntries) {
    // TODO: Properly resolve mapping values from context
    // For now, just use the raw value
    if (typeof mappingValue === 'string') {
      if (mappingValue.startsWith('0x')) {
        values.push(mappingValue);
      } else if (/^\d+$/.test(mappingValue)) {
        values.push(BigInt(mappingValue));
      } else {
        // Expression reference - needs resolver
        values.push(mappingValue);
      }
    } else if (typeof mappingValue === 'number') {
      values.push(BigInt(mappingValue));
    } else {
      values.push(mappingValue);
    }
  }

  // Build calldata
  const signature = buildFunctionSignature(spec.function, types);
  const data = encodeFunctionCall(signature, types, values);

  return {
    to,
    data,
    value: 0n, // TODO: Support value from spec
    chainId: parseChainId(chain),
    stepId,
    stepDescription,
  };
}

/**
 * Build transactions from an action's execution spec
 */
export function buildTransaction(
  protocol: ProtocolSpec,
  action: Action,
  inputs: Record<string, unknown>,
  ctx: ResolverContext,
  options: BuildOptions
): BuildOutput {
  try {
    const { chain } = options;

    // Match execution spec for chain
    const execSpec = matchChainPattern(chain, action.execution);
    if (!execSpec) {
      return {
        success: false,
        error: `No execution spec found for chain "${chain}"`,
      };
    }

    // Resolve input params
    const params = action.params ?? [];
    const resolvedParams: Record<string, unknown> = {};

    for (const param of params) {
      const inputValue = inputs[param.name];

      if (inputValue === undefined && param.required && param.default === undefined) {
        return {
          success: false,
          error: `Missing required parameter: ${param.name}`,
        };
      }

      try {
        resolvedParams[param.name] = resolveParamValue(param, inputValue, ctx);
      } catch (err) {
        return {
          success: false,
          error: `Failed to resolve parameter ${param.name}: ${err instanceof Error ? err.message : err}`,
        };
      }
    }

    // Build transactions based on execution type
    const transactions: TransactionRequest[] = [];

    switch (execSpec.type) {
      case 'evm_call':
      case 'evm_read': {
        const tx = buildEvmCall(protocol, execSpec, ctx, chain);
        transactions.push(tx);
        break;
      }

      case 'composite': {
        // Build each step
        for (const step of (execSpec as Composite).steps) {
          // TODO: Evaluate step.condition
          const stepSpec: EvmCall = {
            type: 'evm_call',
            contract: step.contract,
            function: step.function,
            abi: step.abi,
            mapping: step.mapping,
          };
          const tx = buildEvmCall(
            protocol,
            stepSpec,
            ctx,
            chain,
            step.id,
            step.description
          );
          transactions.push(tx);
        }
        break;
      }

      default:
        return {
          success: false,
          error: `Execution type "${execSpec.type}" not yet implemented`,
        };
    }

    return {
      success: true,
      transactions,
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
  query: Query,
  inputs: Record<string, unknown>,
  ctx: ResolverContext,
  options: BuildOptions
): BuildOutput {
  try {
    const { chain } = options;

    // Match execution spec for chain
    const execSpec = matchChainPattern(chain, query.execution);
    if (!execSpec) {
      return {
        success: false,
        error: `No execution spec found for chain "${chain}"`,
      };
    }

    if (execSpec.type !== 'evm_read') {
      return {
        success: false,
        error: `Query execution type must be evm_read, got "${execSpec.type}"`,
      };
    }

    const tx = buildEvmCall(protocol, execSpec, ctx, chain);

    // Create a pseudo-action for the result type
    const pseudoAction: Action = {
      description: query.description,
      risk_level: 1,
      execution: query.execution,
      params: query.params,
      returns: query.returns,
    };

    return {
      success: true,
      transactions: [{ ...tx, value: 0n }],
      action: pseudoAction,
      resolvedParams: inputs,
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
