/**
 * Execution module - build transaction calldata from AIS actions
 */

export {
  buildTransaction,
  buildQuery,
  buildWorkflowTransactions,
  type TransactionRequest,
  type BuildOptions,
  type BuildResult,
  type BuildError,
  type BuildOutput,
} from './builder.js';

export {
  encodeFunctionSelector,
  encodeJsonAbiFunctionCall,
  decodeJsonAbiFunctionResult,
  buildFunctionSignatureFromJsonAbi,
  AbiArgsError,
  AbiEncodingError,
  AbiDecodingError,
  keccak256,
  compileEvmExecution,
  compileEvmExecutionAsync,
  compileEvmCall,
  compileEvmCallAsync,
  compileEvmRead,
  compileEvmReadAsync,
  compileEvmGetBalance,
  compileEvmGetBalanceAsync,
  type CompileEvmOptions,
  type CompiledEvmRequest,
  type CompiledEvmAbiRequest,
  type CompiledEvmGetBalanceRequest,
  EvmCompileError,
} from './evm/index.js';

// Solana execution
export * as solana from './solana/index.js';
export * as evm from './evm/index.js';

// Execution plan IR
export {
  ExecutionPlanSchema,
  ExecutionPlanNodeSchema,
  buildWorkflowExecutionPlan,
  selectExecutionSpec,
  getNodeReadiness,
  getNodeReadinessAsync,
  type ExecutionPlan,
  type ExecutionPlanNode,
  type PlanWrite,
  type NodeReadinessResult,
  type NodeRunState,
  PlanBuildError,
} from './plan.js';
