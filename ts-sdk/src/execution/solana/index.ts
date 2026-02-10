/**
 * Solana execution helpers (AIS 0.0.2)
 *
 * Uses `@solana/web3.js` and `@solana/spl-token` instead of custom crypto/types.
 */

export { PublicKey, TransactionInstruction, SystemProgram } from '@solana/web3.js';
export type { AccountMeta } from '@solana/web3.js';

export {
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountIdempotentInstruction,
  createTransferInstruction,
  createTransferCheckedInstruction,
  createApproveInstruction,
} from '@solana/spl-token';

export {
  compileSolanaInstruction,
  compileSolanaInstructionAsync,
  createDefaultSolanaInstructionCompilerRegistry,
  type CompileSolanaOptions,
  type CompiledSolanaInstructionRequest,
  SolanaCompileError,
} from './compiler.js';

export {
  SolanaInstructionCompilerRegistry,
  type SolanaInstructionCompiler,
  type SolanaInstructionCompilerContext,
  type SolanaCompiledAccount,
} from './registry.js';
