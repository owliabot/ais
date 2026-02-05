/**
 * Execution Builder - build transaction calldata from AIS actions
 * 
 * Supports AIS-2 execution types with:
 * - Chain pattern matching (eip155:*, solana:*, etc.)
 * - CEL expression evaluation in mapping values
 * - Condition evaluation for composite steps
 * - Contract resolution from deployments
 * - Detect object handling (structured detection)
 */

import type {
  Action,
  Query,
  ProtocolSpec,
  Param,
  ExecutionSpec,
  EvmCall,
  EvmRead,
  EvmMultiread,
  Composite,
  CompositeStep,
  Detect,
} from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { Evaluator, type CELValue, type CELContext } from '../cel/evaluator.js';
import { getContractAddress } from '../resolver/reference.js';
import { resolveExpressionString, hasExpressions } from '../resolver/expression.js';
import { encodeFunctionCall, buildFunctionSignature } from './encoder.js';
import {
  buildPreAuthorize,
  type PreAuthorizeContext,
  type PreAuthorizeResult,
  type PermitData,
} from './pre-authorize.js';

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
  /** Pre-authorization context (wallet address, allowances, nonces) */
  preAuthorize?: PreAuthorizeContext;
}

export interface BuildResult {
  success: true;
  transactions: TransactionRequest[];
  action: Action;
  resolvedParams: Record<string, unknown>;
  calculatedValues: Record<string, unknown>;
  /** Pre-authorization result (if pre_authorize was specified) */
  preAuthorize?: PreAuthorizeResult;
  /** Permit signature data (for permit/permit2 methods) */
  permitData?: PermitData;
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

  // Keep float/token_amount as numbers for CEL processing
  // They'll be converted to atomic amounts by to_atomic() or similar
  if (param.type === 'float' || param.type === 'token_amount') {
    if (typeof value === 'string') {
      return parseFloat(value);
    }
    if (typeof value === 'number') {
      return value;
    }
    throw new Error(`Invalid ${param.type} for ${param.name}: ${value}`);
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
      // Only convert to BigInt if it's an integer
      if (Number.isInteger(value)) {
        return BigInt(value);
      }
      throw new Error(`Invalid integer for ${param.name}: ${value} (got float)`);
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
// CEL Expression Detection
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Check if a value contains a CEL function call or complex expression
 * CEL expressions: function calls, operators, ternary
 */
function isCELExpression(value: string): boolean {
  // Function calls: to_atomic(...), floor(...), etc.
  if (/\w+\s*\(/.test(value)) return true;
  // Operators: +, -, *, /, <, >, ==, etc.
  if (/[+\-*/%<>=!&|?:]/.test(value)) return true;
  return false;
}

/**
 * Build CEL context from ResolverContext
 * Flattens nested objects for CEL evaluation
 */
/**
 * Convert a value to CEL-compatible type
 * BigInt is converted to number for CEL calculations
 */
function toCELValue(value: unknown): CELValue {
  if (typeof value === 'bigint') {
    // Convert to number - safe for values up to Number.MAX_SAFE_INTEGER
    // For very large values, precision may be lost but CEL comparison will still work
    return Number(value);
  }
  return value as CELValue;
}

function buildCELContext(
  ctx: ResolverContext,
  protocol: ProtocolSpec,
  chain: string
): CELContext {
  const celCtx: CELContext = {};

  // Build nested params object for CEL member access
  const params: Record<string, CELValue> = {};
  const calculated: Record<string, CELValue> = {};
  const ctxVars: Record<string, CELValue> = {};

  // Parse variables into nested namespaces
  for (const [key, value] of Object.entries(ctx.variables)) {
    const celValue = toCELValue(value);
    const parts = key.split('.');
    if (parts[0] === 'params' && parts.length >= 2) {
      // Build nested structure for params.token_in.address etc.
      let current = params;
      for (let i = 1; i < parts.length - 1; i++) {
        if (!(parts[i] in current)) {
          current[parts[i]] = {};
        }
        current = current[parts[i]] as Record<string, CELValue>;
      }
      current[parts[parts.length - 1]] = celValue;
    } else if (parts[0] === 'calculated' && parts.length >= 2) {
      calculated[parts.slice(1).join('.')] = celValue;
    } else if (parts[0] === 'ctx' && parts.length >= 2) {
      ctxVars[parts.slice(1).join('.')] = celValue;
    } else {
      // Flat key
      celCtx[key] = celValue;
    }
  }

  // Set namespace objects
  if (Object.keys(params).length > 0) {
    celCtx['params'] = params;
  }
  if (Object.keys(calculated).length > 0) {
    celCtx['calculated'] = calculated;
  }
  if (Object.keys(ctxVars).length > 0) {
    celCtx['ctx'] = ctxVars;
  }

  // Build nested query object with bigint conversion
  const query: Record<string, CELValue> = {};
  for (const [queryName, result] of ctx.queryResults) {
    // Convert query result values (may contain bigints)
    const converted: Record<string, CELValue> = {};
    for (const [k, v] of Object.entries(result)) {
      converted[k] = toCELValue(v);
    }
    query[queryName] = converted;
  }
  if (Object.keys(query).length > 0) {
    celCtx['query'] = query;
  }

  // Build contracts object from protocol deployments
  const contracts: Record<string, CELValue> = {};
  const deployment = protocol.deployments?.find((d) => {
    if (d.chain === chain) return true;
    // Wildcard match: eip155:* matches eip155:1
    const [ns] = chain.split(':');
    return d.chain === `${ns}:*`;
  });

  if (deployment?.contracts) {
    for (const [name, address] of Object.entries(deployment.contracts)) {
      contracts[name] = address;
    }
  }
  if (Object.keys(contracts).length > 0) {
    celCtx['contracts'] = contracts;
  }

  return celCtx;
}

// ═══════════════════════════════════════════════════════════════════════════════
// Mapping Resolution
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Resolve a mapping value from context
 * Supports:
 * - Literal values: "0x...", "123", "true", "false", "null"
 * - References: params.*, calculated.*, ctx.*, query.*, contracts.*
 * - CEL expressions: to_atomic(params.amount, params.token), floor(x * 0.99)
 * - Detect objects: { detect: { kind: "best_quote", ... } }
 */
function resolveMappingValue(
  value: unknown,
  ctx: ResolverContext,
  celCtx: CELContext,
  evaluator: Evaluator,
  protocol: ProtocolSpec,
  chain: string
): unknown {
  // Handle detect objects
  if (value && typeof value === 'object' && 'detect' in value) {
    return resolveDetect((value as { detect: Detect }).detect, ctx, protocol, chain);
  }

  if (typeof value !== 'string') {
    // Not a string - return as-is (number, object, etc.)
    if (typeof value === 'number') {
      return BigInt(value);
    }
    return value;
  }

  // Check for literal values
  if (value.startsWith('0x')) {
    return value; // Address or hex data
  }
  if (/^-?\d+$/.test(value)) {
    return BigInt(value); // Integer literal
  }
  if (value === 'true') return true;
  if (value === 'false') return false;
  if (value === 'null') return null;

  // Check if it's a CEL expression (function call or operators)
  if (isCELExpression(value)) {
    try {
      const result = evaluator.evaluate(value, celCtx);
      // Convert numeric results to BigInt for transaction encoding
      if (typeof result === 'number') {
        return BigInt(Math.floor(result));
      }
      if (typeof result === 'string' && /^\d+$/.test(result)) {
        return BigInt(result);
      }
      return result;
    } catch (err) {
      throw new Error(
        `CEL evaluation failed for "${value}": ${err instanceof Error ? err.message : err}`
      );
    }
  }

  // Reference patterns: params.*, calculated.*, ctx.*, query.*, contracts.*
  const parts = value.split('.');
  const namespace = parts[0];

  switch (namespace) {
    case 'params': {
      // params.token_in or params.token_in.address
      const key = parts.slice(1).join('.');
      const paramValue = ctx.variables[`params.${key}`];
      if (paramValue !== undefined) return paramValue;
      // Try without 'params.' prefix
      return ctx.variables[key];
    }
    case 'calculated': {
      const key = parts.slice(1).join('.');
      return ctx.variables[`calculated.${key}`];
    }
    case 'ctx': {
      const key = parts.slice(1).join('.');
      return ctx.variables[`ctx.${key}`];
    }
    case 'query': {
      const queryName = parts[1];
      const field = parts.slice(2).join('.');
      const result = ctx.queryResults.get(queryName);
      if (!result) return undefined;
      return field ? result[field] : result;
    }
    case 'contracts': {
      // contracts.router - resolve from protocol deployments
      const contractName = parts[1];
      const addr = getContractAddress(protocol, chain, contractName);
      if (!addr) {
        throw new Error(`Contract "${contractName}" not found for chain "${chain}"`);
      }
      return addr;
    }
    default:
      // Unknown reference - return as-is
      return value;
  }
}

/**
 * Resolve a detect object
 * Detect objects specify dynamic value resolution (e.g., best quote, choose one)
 */
function resolveDetect(
  detect: Detect,
  ctx: ResolverContext,
  protocol: ProtocolSpec,
  chain: string
): unknown {
  switch (detect.kind) {
    case 'choose_one':
      // For choose_one, return first candidate or use provider
      if (detect.candidates && detect.candidates.length > 0) {
        return detect.candidates[0];
      }
      throw new Error('choose_one detect requires candidates');

    case 'best_quote':
    case 'best_path':
      // These require async provider calls - return placeholder for now
      // In production, engine would query the provider
      if (detect.candidates && detect.candidates.length > 0) {
        // Return first candidate as fallback
        return detect.candidates[0];
      }
      throw new Error(`${detect.kind} detect requires provider implementation`);

    case 'protocol_specific':
      // Protocol-specific detection - requires protocol handler
      throw new Error('protocol_specific detect not yet implemented');

    default:
      throw new Error(`Unknown detect kind: ${detect.kind}`);
  }
}

/**
 * Evaluate a condition expression (CEL)
 * Returns true if condition passes (or is undefined), false to skip step
 */
function evaluateCondition(
  condition: string | undefined,
  celCtx: CELContext,
  evaluator: Evaluator
): boolean {
  if (!condition) return true;

  try {
    const result = evaluator.evaluate(condition.trim(), celCtx);
    return Boolean(result);
  } catch (err) {
    throw new Error(
      `Condition evaluation failed: ${err instanceof Error ? err.message : err}`
    );
  }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Calculated Fields
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Topological sort for calculated field dependencies
 * Returns field names in evaluation order
 */
function topologicalSortFields(
  fields: Record<string, { expr: string; inputs?: string[] }>
): string[] {
  const result: string[] = [];
  const visited = new Set<string>();
  const visiting = new Set<string>();

  // Build dependency graph from inputs
  const deps = new Map<string, string[]>();
  for (const [name, field] of Object.entries(fields)) {
    const calcDeps: string[] = [];
    for (const input of field.inputs ?? []) {
      if (input.startsWith('calculated.')) {
        calcDeps.push(input.slice('calculated.'.length));
      }
    }
    deps.set(name, calcDeps);
  }

  function visit(name: string): void {
    if (visited.has(name)) return;
    if (visiting.has(name)) {
      throw new Error(`Circular dependency in calculated_fields: ${name}`);
    }

    visiting.add(name);
    for (const dep of deps.get(name) ?? []) {
      if (dep in fields) {
        visit(dep);
      }
    }
    visiting.delete(name);
    visited.add(name);
    result.push(name);
  }

  for (const name of Object.keys(fields)) {
    visit(name);
  }

  return result;
}

/**
 * Compute all calculated_fields for an action
 * Evaluates in dependency order and returns computed values
 */
function computeCalculatedFields(
  action: Action,
  ctx: ResolverContext,
  protocol: ProtocolSpec,
  chain: string,
  evaluator: Evaluator
): Record<string, CELValue> {
  const calcFields = action.calculated_fields;
  if (!calcFields || Object.keys(calcFields).length === 0) {
    return {};
  }

  // Get evaluation order
  const order = topologicalSortFields(calcFields);
  const computed: Record<string, CELValue> = {};

  // Evaluate each field in order
  for (const fieldName of order) {
    const field = calcFields[fieldName];
    
    // Build fresh CEL context with current computed values
    const celCtx = buildCELContext(ctx, protocol, chain);
    
    // Add already-computed calculated fields
    if (!celCtx['calculated']) {
      celCtx['calculated'] = {};
    }
    for (const [k, v] of Object.entries(computed)) {
      (celCtx['calculated'] as Record<string, CELValue>)[k] = v;
    }

    try {
      const result = evaluator.evaluate(field.expr, celCtx);
      computed[fieldName] = result as CELValue;
      
      // Also set in resolver context for mapping resolution
      ctx.variables[`calculated.${fieldName}`] = result as CELValue;
    } catch (err) {
      throw new Error(
        `Failed to compute calculated_field "${fieldName}": ${err instanceof Error ? err.message : err}`
      );
    }
  }

  return computed;
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
  celCtx: CELContext,
  evaluator: Evaluator,
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
    const resolved = resolveMappingValue(contractRef, ctx, celCtx, evaluator, protocol, chain);
    if (typeof resolved !== 'string' || !resolved.startsWith('0x')) {
      throw new Error(`Contract reference "${contractRef}" did not resolve to address`);
    }
    to = resolved;
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

  // Resolve mapping values from context with CEL support
  const values: unknown[] = [];
  const mappingEntries = Object.entries(spec.mapping);
  
  for (const [, mappingValue] of mappingEntries) {
    const resolved = resolveMappingValue(mappingValue, ctx, celCtx, evaluator, protocol, chain);
    values.push(resolved);
  }

  // Build calldata
  const signature = buildFunctionSignature(spec.function, types);
  const data = encodeFunctionCall(signature, types, values);

  // Resolve value if specified (for payable functions)
  let txValue = 0n;
  if (spec.type === 'evm_call' && spec.value) {
    const resolvedValue = resolveMappingValue(spec.value, ctx, celCtx, evaluator, protocol, chain);
    if (typeof resolvedValue === 'bigint') {
      txValue = resolvedValue;
    } else if (typeof resolvedValue === 'string' && /^\d+$/.test(resolvedValue)) {
      txValue = BigInt(resolvedValue);
    } else if (typeof resolvedValue === 'number') {
      txValue = BigInt(resolvedValue);
    }
  }

  return {
    to,
    data,
    value: txValue,
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

    // Create CEL evaluator
    const evaluator = new Evaluator();

    // Resolve input params and set them in context
    const params = action.params ?? [];
    const resolvedParams: Record<string, unknown> = {};

    for (const param of params) {
      const inputValue = inputs[param.name];

      // Check required (default to true if not specified)
      const isRequired = param.required !== false;
      if (inputValue === undefined && isRequired && param.default === undefined) {
        return {
          success: false,
          error: `Missing required parameter: ${param.name}`,
        };
      }

      try {
        const resolved = resolveParamValue(param, inputValue, ctx);
        resolvedParams[param.name] = resolved;
        // Set in context for mapping resolution
        ctx.variables[`params.${param.name}`] = resolved as CELValue;
      } catch (err) {
        return {
          success: false,
          error: `Failed to resolve parameter ${param.name}: ${err instanceof Error ? err.message : err}`,
        };
      }
    }

    // Compute calculated_fields (must be after params are resolved)
    let calculatedValues: Record<string, CELValue> = {};
    if (action.calculated_fields) {
      try {
        calculatedValues = computeCalculatedFields(action, ctx, protocol, chain, evaluator);
      } catch (err) {
        return {
          success: false,
          error: err instanceof Error ? err.message : String(err),
        };
      }
    }

    // Build CEL context with all resolved values (including calculated)
    const celCtx = buildCELContext(ctx, protocol, chain);

    // Build transactions based on execution type
    const transactions: TransactionRequest[] = [];
    let preAuthorizeResult: PreAuthorizeResult | undefined;
    let permitData: PermitData | undefined;

    switch (execSpec.type) {
      case 'evm_call': {
        // Handle pre_authorize if specified
        if (execSpec.pre_authorize && options.preAuthorize) {
          try {
            preAuthorizeResult = buildPreAuthorize(
              execSpec.pre_authorize,
              ctx,
              celCtx,
              evaluator,
              protocol,
              chain,
              options.preAuthorize
            );

            // Add approval transactions if needed
            if (preAuthorizeResult.needed) {
              // Permit2 approval tx first (if needed)
              if (preAuthorizeResult.permit2ApproveTx) {
                transactions.push(preAuthorizeResult.permit2ApproveTx);
              }
              // Standard approve tx (if needed)
              if (preAuthorizeResult.approveTx) {
                transactions.push(preAuthorizeResult.approveTx);
              }
              // Save permit data for caller to sign
              if (preAuthorizeResult.permitData) {
                permitData = preAuthorizeResult.permitData;
              }
            }
          } catch (err) {
            return {
              success: false,
              error: `Pre-authorization failed: ${err instanceof Error ? err.message : err}`,
            };
          }
        }

        // Build main transaction
        const tx = buildEvmCall(protocol, execSpec, ctx, celCtx, evaluator, chain);
        transactions.push(tx);
        break;
      }

      case 'evm_read': {
        const tx = buildEvmCall(protocol, execSpec, ctx, celCtx, evaluator, chain);
        transactions.push(tx);
        break;
      }

      case 'composite': {
        // Build each step, evaluating conditions
        for (const step of (execSpec as Composite).steps) {
          // Evaluate step condition
          if (!evaluateCondition(step.condition, celCtx, evaluator)) {
            // Condition is false, skip this step
            continue;
          }

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
            celCtx,
            evaluator,
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
      calculatedValues,
      ...(preAuthorizeResult && { preAuthorize: preAuthorizeResult }),
      ...(permitData && { permitData }),
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

    if (execSpec.type !== 'evm_read' && execSpec.type !== 'evm_multiread') {
      return {
        success: false,
        error: `Query execution type must be evm_read or evm_multiread, got "${execSpec.type}"`,
      };
    }

    // Create CEL evaluator
    const evaluator = new Evaluator();

    // Resolve params and set in context
    const params = query.params ?? [];
    const resolvedParams: Record<string, unknown> = {};

    for (const param of params) {
      const inputValue = inputs[param.name];
      const isRequired = param.required !== false;
      
      if (inputValue === undefined && isRequired && param.default === undefined) {
        return {
          success: false,
          error: `Missing required parameter: ${param.name}`,
        };
      }

      try {
        const resolved = resolveParamValue(param, inputValue, ctx);
        resolvedParams[param.name] = resolved;
        ctx.variables[`params.${param.name}`] = resolved as CELValue;
      } catch (err) {
        return {
          success: false,
          error: `Failed to resolve parameter ${param.name}: ${err instanceof Error ? err.message : err}`,
        };
      }
    }

    // Build CEL context
    const celCtx = buildCELContext(ctx, protocol, chain);

    // Handle evm_multiread differently
    if (execSpec.type === 'evm_multiread') {
      // Build multiple calls for multiread
      const transactions: TransactionRequest[] = [];
      for (const call of execSpec.calls) {
        const callSpec: EvmRead = {
          type: 'evm_read',
          contract: call.contract,
          function: call.function,
          abi: call.abi,
          mapping: call.mapping,
        };
        const tx = buildEvmCall(protocol, callSpec, ctx, celCtx, evaluator, chain);
        transactions.push({ ...tx, stepId: call.output_as });
      }

      const pseudoAction: Action = {
        description: query.description,
        risk_level: 1,
        execution: query.execution,
        params: query.params,
        returns: query.returns,
      };

      return {
        success: true,
        transactions,
        action: pseudoAction,
        resolvedParams,
        calculatedValues: {},
      };
    }

    // Single evm_read
    const tx = buildEvmCall(protocol, execSpec, ctx, celCtx, evaluator, chain);

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
      resolvedParams,
      calculatedValues: {},
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
