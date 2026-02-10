import type { ResolverContext } from '../../resolver/index.js';
import type { SolanaInstruction } from '../../schema/index.js';
import type { PublicKey, TransactionInstruction } from '@solana/web3.js';

export interface SolanaCompiledAccount {
  name: string;
  pubkey: PublicKey;
  isSigner: boolean;
  isWritable: boolean;
}

export interface SolanaInstructionCompilerContext {
  execution: SolanaInstruction;
  ctx: ResolverContext;
  chain: string;
  programId: PublicKey;
  instruction: string;
  accounts: SolanaCompiledAccount[];
  accountMap: Map<string, SolanaCompiledAccount>;
  data: unknown;
  discriminator?: unknown;

  getAccount(...names: string[]): SolanaCompiledAccount;
}

export type SolanaInstructionCompiler = (
  ctx: SolanaInstructionCompilerContext
) => TransactionInstruction;

export class SolanaInstructionCompilerRegistry {
  private readonly byProgram: Map<string, Map<string, SolanaInstructionCompiler>> = new Map();

  clone(): SolanaInstructionCompilerRegistry {
    const out = new SolanaInstructionCompilerRegistry();
    for (const [program, byIx] of this.byProgram.entries()) {
      for (const [ix, compiler] of byIx.entries()) {
        out.register(program, ix, compiler);
      }
    }
    return out;
  }

  register(
    programId: string | PublicKey,
    instruction: string,
    compiler: SolanaInstructionCompiler
  ): void {
    const programKey = typeof programId === 'string' ? programId : programId.toBase58();
    const byIx = this.byProgram.get(programKey) ?? new Map<string, SolanaInstructionCompiler>();
    byIx.set(instruction, compiler);
    this.byProgram.set(programKey, byIx);
  }

  get(
    programId: string | PublicKey,
    instruction: string
  ): SolanaInstructionCompiler | undefined {
    const programKey = typeof programId === 'string' ? programId : programId.toBase58();
    return this.byProgram.get(programKey)?.get(instruction);
  }

  list(): Array<{ program_id: string; instruction: string }> {
    const out: Array<{ program_id: string; instruction: string }> = [];
    for (const [program, byIx] of this.byProgram.entries()) {
      for (const ix of byIx.keys()) out.push({ program_id: program, instruction: ix });
    }
    return out.sort((a, b) =>
      a.program_id === b.program_id ? a.instruction.localeCompare(b.instruction) : a.program_id.localeCompare(b.program_id)
    );
  }
}
