import { formatPlanSummary } from './plan-print.js';
import type {
  DryRunSdk,
  RunnerContext,
  RunnerPlan,
  RunnerPlanNode,
  RunnerReadiness,
  RunnerSolanaInstruction,
} from './types.js';

export async function dryRunCompilePlan(args: {
  sdk: DryRunSdk;
  plan: RunnerPlan;
  ctx: RunnerContext;
}): Promise<string> {
  const { sdk, plan, ctx } = args;
  const solver = sdk.createSolver ? sdk.createSolver() : sdk.solver;

  const lines: string[] = [];
  lines.push('== dry-run (compile only) ==');
  lines.push(formatPlanSummary(plan).trimEnd());
  lines.push('');

  for (const node of plan.nodes ?? []) {
    lines.push(`# node ${node.id}`);
    const r1 = sdk.getNodeReadiness(node, ctx);
    const r2 = await maybeSolveAndRecheck(sdk, solver, node, r1, ctx);
    if (r2.state !== 'ready') {
      lines.push(`state=${r2.state}`);
      if (r2.missing_refs?.length) lines.push(`missing_refs=${r2.missing_refs.join(',')}`);
      if (r2.needs_detect) lines.push('needs_detect=true');
      if (r2.errors?.length) lines.push(`errors=${r2.errors.join('; ')}`);
      lines.push('');
      continue;
    }

    lines.push('state=ready');

    const resolvedParams = r2.resolved_params ?? {};
    try {
      lines.push(...compileNode(sdk, node, ctx, resolvedParams));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      lines.push(`compile_error=${msg}`);
    }
    lines.push('');
  }

  return `${lines.join('\n')}\n`;
}

async function maybeSolveAndRecheck(
  sdk: DryRunSdk,
  solver: DryRunSdk['solver'],
  node: RunnerPlanNode,
  readiness: RunnerReadiness,
  ctx: RunnerContext
): Promise<RunnerReadiness> {
  if (readiness.state !== 'blocked') return readiness;
  const solved = await solver.solve(node, readiness, ctx);
  const patches = Array.isArray(solved?.patches) ? solved.patches : [];
  if (patches.length > 0) {
    sdk.applyRuntimePatches(ctx, patches);
  }
  return sdk.getNodeReadiness(node, ctx);
}

function compileNode(
  sdk: DryRunSdk,
  node: RunnerPlanNode,
  ctx: RunnerContext,
  resolvedParams: Record<string, unknown>
): string[] {
  const exec = node.execution;
  if (!exec || typeof exec !== 'object') return ['exec_type=unknown'];
  const t = String(exec.type ?? '');
  if (t === 'evm_call' || t === 'evm_read' || t === 'evm_rpc') {
    const compiled = sdk.compileEvmExecution(exec, ctx, { chain: node.chain, params: resolvedParams });
    if (compiled.kind === 'evm_rpc') {
      return [
        `exec_type=${t}`,
        `chain=${compiled.chain} chainId=${compiled.chainId}`,
        `method=${compiled.method}`,
        `params=${JSON.stringify(compiled.params)}`,
      ];
    }
    return [
      `exec_type=${t}`,
      `chain=${compiled.chain} chainId=${compiled.chainId}`,
      `to=${compiled.to}`,
      `value=${String(compiled.value)}`,
      `data=${compiled.data}`,
      `abi=${compiled.abi?.name ?? ''}`,
    ];
  }
  if (t === 'solana_instruction') {
    if (!sdk.solana?.compileSolanaInstruction) return ['exec_type=solana_instruction', 'compile_error=missing solana compiler'];
    if (!isSolanaInstructionExecution(exec)) return ['exec_type=solana_instruction', 'compile_error=invalid execution spec'];
    const compiled = sdk.solana.compileSolanaInstruction(exec, ctx, { chain: node.chain, params: resolvedParams });
    const program =
      typeof compiled.programId === 'object' && compiled.programId !== null && typeof compiled.programId.toBase58 === 'function'
        ? compiled.programId.toBase58()
        : String(compiled.programId ?? '');
    return [
      `exec_type=${t}`,
      `chain=${node.chain}`,
      `program=${program}`,
      `instruction=${String(compiled.instruction ?? '')}`,
      `lookup_tables=${Array.isArray(compiled.lookupTables) ? compiled.lookupTables.length : 0}`,
    ];
  }
  if (t === 'solana_read') {
    const method = String((exec as { method?: unknown }).method ?? '');
    return [`exec_type=${t}`, `chain=${node.chain}`, `method=${method}`];
  }
  return [`exec_type=${t}`];
}

function isSolanaInstructionExecution(exec: RunnerPlanNode['execution']): exec is RunnerSolanaInstruction {
  return String(exec?.type ?? '') === 'solana_instruction';
}
