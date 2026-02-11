import type { RunnerContext, RunnerPlanNode, RunnerSolanaInstruction } from '../../../types.js';
import type { WrapperSdk } from './types.js';

export function compileWritePreview(
  sdk: WrapperSdk,
  node: RunnerPlanNode,
  ctx: RunnerContext,
  resolvedParams: Record<string, unknown>
): unknown {
  const execType = node.execution.type;
  try {
    if ((execType === 'evm_call' || execType === 'evm_multicall') && typeof sdk.compileEvmExecution === 'function') {
      const compiled = sdk.compileEvmExecution(node.execution, ctx, { chain: node.chain, params: resolvedParams });
      return {
        chain: compiled.chain,
        chainId: compiled.chainId,
        to: 'to' in compiled ? compiled.to : undefined,
        data: 'data' in compiled ? compiled.data : undefined,
        value: 'value' in compiled ? String(compiled.value) : undefined,
        abi: 'abi' in compiled ? compiled.abi?.name : undefined,
      };
    }
    if (
      execType === 'solana_instruction' &&
      sdk.solana?.compileSolanaInstruction &&
      isSolanaInstructionExecution(node.execution)
    ) {
      const compiled = sdk.solana.compileSolanaInstruction(node.execution, ctx, {
        chain: node.chain,
        params: resolvedParams,
      });
      const program = compiled.programId?.toBase58 ? compiled.programId.toBase58() : String(compiled.programId ?? '');
      return {
        chain: node.chain,
        program,
        instruction: compiled.instruction,
        lookup_tables: compiled.lookupTables ?? [],
        compute_units: compiled.computeUnits ?? undefined,
      };
    }
  } catch (error) {
    return { chain: node.chain, exec_type: execType, compile_error: (error as Error)?.message ?? String(error) };
  }
  return { chain: node.chain, exec_type: execType };
}

function isSolanaInstructionExecution(execution: RunnerPlanNode['execution']): execution is RunnerSolanaInstruction {
  return execution.type === 'solana_instruction';
}
