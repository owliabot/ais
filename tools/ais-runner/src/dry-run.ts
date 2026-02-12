import { formatPlanSummary } from './plan-print.js';
import type {
  DryRunSdk,
  RunnerContext,
  RunnerPack,
  RunnerPlan,
  RunnerPlanNode,
  RunnerReadiness,
  RunnerSolanaInstruction,
} from './types.js';

export type DryRunCompileNodeReport = {
  id: string;
  state: string;
  missing_refs?: string[];
  needs_detect?: boolean;
  errors?: string[];
  compile?: {
    exec_type: string;
    details?: Record<string, unknown>;
    error?: string;
  };
  policy_gate?: {
    input?: unknown;
    result?: unknown;
    skipped?: boolean;
  };
};

export type DryRunCompilePlanJson = {
  kind: 'dry_run_compile_plan';
  plan_summary: string;
  nodes: DryRunCompileNodeReport[];
  issues: Array<{
    kind: string;
    severity: 'error' | 'warning' | 'info';
    node_id?: string;
    field_path: string;
    message: string;
    reference?: string;
    related?: { path?: string; node_id?: string; field_path?: string; reference?: string };
  }>;
};

// Local helper: keep StructuredIssue-like shape without coupling runner build to a specific sdk version.
function zodToStructuredIssues(error: any, opts?: { kind?: string; severity?: 'error' | 'warning' | 'info' }): DryRunCompilePlanJson['issues'] {
  const kind = opts?.kind ?? 'schema_validation';
  const severity = opts?.severity ?? 'error';
  const zissues = Array.isArray(error?.issues) ? error.issues : [];
  return zissues.map((i: any) => ({
    kind,
    severity,
    field_path: Array.isArray(i?.path) ? zodPathToFieldPath(i.path) : '(root)',
    message: String(i?.message ?? 'Invalid value'),
    reference: typeof i?.code === 'string' ? i.code : undefined,
  }));
}

function zodPathToFieldPath(path: Array<string | number>): string {
  if (path.length === 0) return '(root)';
  let out = '';
  for (const seg of path) {
    if (typeof seg === 'number') {
      out += `[${seg}]`;
      continue;
    }
    if (!out) out = seg;
    else out += `.${seg}`;
  }
  return out;
}

export async function dryRunCompilePlan(args: {
  sdk: DryRunSdk;
  plan: RunnerPlan;
  ctx: RunnerContext;
  pack?: RunnerPack;
}): Promise<string> {
  const { sdk, plan, ctx, pack } = args;
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
    lines.push(...evaluatePolicyGateDryRun({ sdk, node, ctx, pack, resolvedParams }));
    lines.push('');
  }

  return `${lines.join('\n')}\n`;
}

export async function dryRunCompilePlanJson(args: {
  sdk: DryRunSdk;
  plan: RunnerPlan;
  ctx: RunnerContext;
  pack?: RunnerPack;
}): Promise<DryRunCompilePlanJson> {
  const { sdk, plan, ctx, pack } = args;
  const solver = sdk.createSolver ? sdk.createSolver() : sdk.solver;

  const nodes: DryRunCompileNodeReport[] = [];
  const issues: DryRunCompilePlanJson['issues'] = [];

  // Plan schema validation issues are agent-relevant.
  const validation = sdk.ExecutionPlanSchema?.safeParse ? sdk.ExecutionPlanSchema.safeParse(plan) : { success: true };
  if (validation && (validation as any).success === false) {
    issues.push(...zodToStructuredIssues((validation as any).error, { kind: 'plan_validation', severity: 'error' }));
  }

  for (const node of plan.nodes ?? []) {
    const r1 = sdk.getNodeReadiness(node, ctx);
    const r2 = await maybeSolveAndRecheck(sdk, solver, node, r1, ctx);
    const rec: DryRunCompileNodeReport = {
      id: node.id,
      state: r2.state,
    };
    if (r2.missing_refs?.length) rec.missing_refs = r2.missing_refs.slice();
    if (r2.needs_detect) rec.needs_detect = true;
    if (r2.errors?.length) rec.errors = r2.errors.slice();

    if (r2.state === 'ready') {
      const resolvedParams = r2.resolved_params ?? {};

      rec.compile = compileNodeJson(sdk, node, ctx, resolvedParams);

      rec.policy_gate = evaluatePolicyGateDryRunJson({ sdk, node, ctx, pack, resolvedParams });
    }

    nodes.push(rec);
  }

  return {
    kind: 'dry_run_compile_plan',
    plan_summary: formatPlanSummary(plan).trimEnd(),
    nodes,
    issues,
  };
}

function evaluatePolicyGateDryRun(args: {
  sdk: DryRunSdk;
  node: RunnerPlanNode;
  ctx: RunnerContext;
  pack?: RunnerPack;
  resolvedParams: Record<string, unknown>;
}): string[] {
  const { sdk, node, ctx, pack, resolvedParams } = args;
  if (!isWriteNode(node)) return [];
  if (
    typeof sdk.compileWritePreview !== 'function' ||
    typeof sdk.extractPolicyGateInput !== 'function' ||
    typeof sdk.enforcePolicyGate !== 'function'
  ) {
    return ['policy_gate=skipped (sdk methods unavailable)'];
  }

  const protocolRef = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const actionId = typeof node.source?.action === 'string' ? node.source.action : '';
  const resolved = protocolRef && actionId && typeof sdk.resolveAction === 'function'
    ? sdk.resolveAction(ctx, `${protocolRef}/${actionId}`)
    : null;

  const preview = sdk.compileWritePreview({
    node,
    ctx,
    resolved_params: resolvedParams,
  });
  const gateInput = sdk.extractPolicyGateInput({
    node,
    ctx,
    pack,
    resolved_params: resolvedParams,
    action_risk_level: resolved?.action?.risk_level,
    action_risk_tags: resolved?.action?.risk_tags,
    preview,
  });
  const gateResult = sdk.enforcePolicyGate(pack, gateInput);
  const explain =
    typeof sdk.explainPolicyGateResult === 'function'
      ? sdk.explainPolicyGateResult(gateResult)
      : gateResult;

  return [
    `policy_gate_input=${safeStringify(gateInput)}`,
    `policy_gate_result=${safeStringify(explain)}`,
  ];
}

function evaluatePolicyGateDryRunJson(args: {
  sdk: DryRunSdk;
  node: RunnerPlanNode;
  ctx: RunnerContext;
  pack?: RunnerPack;
  resolvedParams: Record<string, unknown>;
}): DryRunCompileNodeReport['policy_gate'] {
  const { sdk, node, ctx, pack, resolvedParams } = args;
  if (!isWriteNode(node)) return { skipped: true };
  if (
    typeof sdk.compileWritePreview !== 'function' ||
    typeof sdk.extractPolicyGateInput !== 'function' ||
    typeof sdk.enforcePolicyGate !== 'function'
  ) {
    return { skipped: true };
  }

  const protocolRef = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const actionId = typeof node.source?.action === 'string' ? node.source.action : '';
  const resolved = protocolRef && actionId && typeof sdk.resolveAction === 'function'
    ? sdk.resolveAction(ctx, `${protocolRef}/${actionId}`)
    : null;

  const preview = sdk.compileWritePreview({
    node,
    ctx,
    resolved_params: resolvedParams,
  });
  const gateInput = sdk.extractPolicyGateInput({
    node,
    ctx,
    pack,
    resolved_params: resolvedParams,
    action_risk_level: resolved?.action?.risk_level,
    action_risk_tags: resolved?.action?.risk_tags,
    preview,
  });
  const gateResult = sdk.enforcePolicyGate(pack, gateInput);
  const explain =
    typeof sdk.explainPolicyGateResult === 'function'
      ? sdk.explainPolicyGateResult(gateResult)
      : gateResult;

  return { input: gateInput, result: explain };
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

function compileNodeJson(
  sdk: DryRunSdk,
  node: RunnerPlanNode,
  ctx: RunnerContext,
  resolvedParams: Record<string, unknown>
): NonNullable<DryRunCompileNodeReport['compile']> {
  // This must never throw: dry-run json should be always producible.
  try {
  const exec = node.execution;
  if (!exec || typeof exec !== 'object') return { exec_type: 'unknown' };
  const t = String(exec.type ?? '');
  if (t === 'evm_call' || t === 'evm_read' || t === 'evm_rpc') {
    const compiled = sdk.compileEvmExecution(exec, ctx, { chain: node.chain, params: resolvedParams });
    if (compiled.kind === 'evm_rpc') {
      return {
        exec_type: t,
        details: {
          chain: compiled.chain,
          chainId: compiled.chainId,
          method: compiled.method,
          params: compiled.params,
        },
      };
    }
    return {
      exec_type: t,
      details: {
        chain: compiled.chain,
        chainId: compiled.chainId,
        to: compiled.to,
        value: String(compiled.value),
        data: compiled.data,
        abi: compiled.abi?.name ?? '',
      },
    };
  }
  if (t === 'solana_instruction') {
    if (!sdk.solana?.compileSolanaInstruction) return { exec_type: t, error: 'missing solana compiler' };
    if (!isSolanaInstructionExecution(exec)) return { exec_type: t, error: 'invalid execution spec' };
    const compiled = sdk.solana.compileSolanaInstruction(exec, ctx, { chain: node.chain, params: resolvedParams });
    const program =
      typeof compiled.programId === 'object' && compiled.programId !== null && typeof compiled.programId.toBase58 === 'function'
        ? compiled.programId.toBase58()
        : String(compiled.programId ?? '');
    return {
      exec_type: t,
      details: {
        chain: node.chain,
        program,
        instruction: String(compiled.instruction ?? ''),
        lookup_tables: Array.isArray(compiled.lookupTables) ? compiled.lookupTables.length : 0,
      },
    };
  }
  if (t === 'solana_read') {
    const method = String((exec as { method?: unknown }).method ?? '');
    return { exec_type: t, details: { chain: node.chain, method } };
  }
  return { exec_type: t };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return { exec_type: String((node as any).execution?.type ?? 'unknown'), error: msg };
  }
}

function isSolanaInstructionExecution(exec: RunnerPlanNode['execution']): exec is RunnerSolanaInstruction {
  return String(exec?.type ?? '') === 'solana_instruction';
}

function isWriteNode(node: RunnerPlanNode): boolean {
  if (node.kind === 'query_ref') return false;
  const t = String(node.execution?.type ?? '');
  if (t === 'evm_read' || t === 'evm_rpc' || t === 'evm_multiread' || t === 'solana_read') return false;
  return true;
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
