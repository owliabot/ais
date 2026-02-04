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
  encodeFunctionCall,
  encodeFunctionSelector,
  encodeValue,
  buildFunctionSignature,
  isDynamicType,
} from './encoder.js';

export { keccak256 } from './keccak.js';
