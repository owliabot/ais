import { parseCliArgs, renderHelp } from './cli.js';
import { loadRunnerConfig } from './config.js';
import { loadSdk } from './sdk.js';
import { formatPlanSummary } from './plan-print.js';
import { dryRunCompilePlan } from './dry-run.js';
import { coerceArgsByParams, coerceWorkflowInputs, ensureCtxNow } from './runtime.js';
import { createExecutorsFromConfig } from './executors.js';
import { FileCheckpointStore } from './checkpoint-store.js';
import { ActionPreflightExecutor, BroadcastGateExecutor, PolicyGateExecutor, StrictSuccessExecutor } from './executor-wrappers.js';
import { createRunnerDetectResolver } from './detect.js';
import { wrapSolverWithCalculatedFields } from './solver-wrappers.js';
import { evaluateWorkflowOutputs, stringifyWithBigInt, writeOutputsJson } from './output.js';

export async function run(argv: string[]): Promise<void> {
  const parsed = parseCliArgs(argv);
  if (parsed.kind === 'help') {
    process.stdout.write(renderHelp());
    return;
  }

  const config = parsed.configPath ? await loadRunnerConfig(parsed.configPath) : null;

  const sdk = await loadSdk();

  const { context, result } = await sdk.loadDirectoryAsContext(parsed.workspaceDir, { recursive: true });
  const wsIssues = sdk.validateWorkspaceReferences(result);

  // Runtime injections (minimal for now).
  if (config?.runtime?.ctx && typeof config.runtime.ctx === 'object') {
    context.runtime.ctx = { ...context.runtime.ctx, ...(config.runtime.ctx as any) };
  }
  ensureCtxNow(context.runtime.ctx);
  if (parsed.kind === 'run_workflow') {
    if (parsed.inputsJson) {
      const inputs = JSON.parse(parsed.inputsJson);
      if (inputs && typeof inputs === 'object') context.runtime.inputs = inputs;
    }
    if (parsed.ctxJson) {
      const ctx = JSON.parse(parsed.ctxJson);
      if (ctx && typeof ctx === 'object') context.runtime.ctx = { ...context.runtime.ctx, ...ctx };
    }

    const workflow = await sdk.loadWorkflow(parsed.filePath);
    if (workflow?.inputs && typeof workflow.inputs === 'object') {
      context.runtime.inputs = coerceWorkflowInputs(workflow.inputs, context.runtime.inputs);
    }

    // Workspace validation (filtered to the selected workflow and its deps).
    const relevantPaths = collectRelevantWorkspacePaths(sdk, result, parsed.filePath, workflow);
    const relevantIssues = wsIssues.filter((i: any) => {
      const p = String(i?.path ?? '');
      const rp = i?.related_path ? String(i.related_path) : '';
      return relevantPaths.has(p) || (rp && relevantPaths.has(rp));
    });
    if (relevantIssues.some((i: any) => i?.severity === 'error')) {
      process.stdout.write(`${JSON.stringify({ kind: 'workspace_errors', issues: relevantIssues }, null, 2)}\n`);
      (globalThis as any).process.exitCode = 1;
      return;
    }

    const wfValidation = sdk.validateWorkflow(workflow, context);
    if (!wfValidation.valid) {
      process.stdout.write(`${JSON.stringify({ kind: 'workflow_errors', issues: wfValidation.issues }, null, 2)}\n`);
      (globalThis as any).process.exitCode = 1;
      return;
    }

    const plan = sdk.buildWorkflowExecutionPlan(workflow, context);
    process.stdout.write(`${JSON.stringify({ request: parsed, config, load_errors: result.errors }, null, 2)}\n\n`);
    if (parsed.dryRun) {
      process.stdout.write(await dryRunCompilePlan({ sdk, plan, ctx: context }));
    } else {
      if (!config) {
        process.stdout.write('Missing --config for execution mode\n');
        process.exitCode = 1;
        return;
      }
      if (parsed.broadcast) {
        const missing = missingSignerChains(plan, config);
        if (missing.length > 0) {
          process.stdout.write(`Missing signer config for broadcast on chains: ${missing.join(', ')}\n`);
          process.exitCode = 1;
          return;
        }
      }

      let baseExecutors: any[];
      try {
        baseExecutors = await createExecutorsFromConfig(sdk, config, { allow_broadcast: Boolean(parsed.broadcast) });
      } catch (e) {
        process.stdout.write(`Failed to create executors from config: ${(e as Error)?.message ?? String(e)}\n`);
        process.exitCode = 1;
        return;
      }
      const packDoc = findRequiredPackDocument(workflow, result);
      const executors = baseExecutors.map(
        (ex) =>
          new StrictSuccessExecutor(
            new BroadcastGateExecutor(
              sdk,
              new ActionPreflightExecutor(
                sdk,
                new PolicyGateExecutor(sdk, ex as any, { pack: packDoc?.document, yes: Boolean(parsed.yes) })
              ),
              Boolean(parsed.broadcast)
            )
          )
      );
      if (executors.length === 0) {
        process.stdout.write('No executors created from config (missing/empty rpc_url?)\n');
        process.exitCode = 1;
        return;
      }
      const detect = createRunnerDetectResolver({ sdk, workflow, workspaceDocs: result });
      const baseSolver = sdk.createSolver ? sdk.createSolver() : sdk.solver;
      const solver = wrapSolverWithCalculatedFields({ sdk, inner: baseSolver, detect });
      const trace = parsed.tracePath ? { sink: sdk.createJsonlTraceSink({ file_path: parsed.tracePath }) } : undefined;
      const checkpoint_store = parsed.checkpointPath ? new FileCheckpointStore(sdk, parsed.checkpointPath) : undefined;
      const opts: any = {
        solver,
        executors,
        detect,
        max_concurrency: config?.engine?.max_concurrency,
        per_chain: config?.engine?.per_chain,
        trace,
        checkpoint_store,
        resume_from_checkpoint: Boolean(parsed.resume),
      };
      process.stdout.write(formatPlanSummary(plan));
      let endedEarly = false;
      for await (const ev of sdk.runPlan(plan, context, opts)) {
        applyRunnerSideEffects(sdk, context, ev);
        process.stdout.write(`${formatEvent(ev)}\n`);
        if (ev.type === 'engine_paused') {
          endedEarly = true;
          break;
        }
        if (ev.type === 'error') {
          endedEarly = true;
          break;
        }
      }

      await destroyExecutors(executors);

      if (!endedEarly) {
        const evaluated = evaluateWorkflowOutputs(sdk, workflow, context);
        const payload = {
          kind: 'workflow_outputs',
          outputs: evaluated.outputs,
          errors: evaluated.errors,
        };
        process.stdout.write(`${stringifyWithBigInt(payload)}\n`);
        if (parsed.outPath) await writeOutputsJson(parsed.outPath, payload);
      }
    }
    return;
  }

  if (parsed.kind === 'run_action') {
    if (!parsed.chain) {
      process.stdout.write('Missing --chain for action mode\n');
      (globalThis as any).process.exitCode = 1;
      return;
    }
    const [skill, action] = splitRef(parsed.actionRef);
    if (!skill || !action) {
      process.stdout.write('Invalid --ref for action mode (expected protocol@ver/<actionId>)\n');
      (globalThis as any).process.exitCode = 1;
      return;
    }
    const args = JSON.parse(parsed.argsJson);
    const resolved = sdk.resolveAction(context, `${skill}/${action}`);
    if (!resolved) {
      process.stdout.write('Action not found in workspace\n');
      process.exitCode = 1;
      return;
    }
    const coercedArgs = coerceArgsByParams(resolved.action.params, args);
    const workflow = synthWorkflow('runner-action', parsed.chain, [
      { id: 'n1', type: 'action_ref', chain: parsed.chain, skill, action, args: toLitValueRefs(coercedArgs) },
    ]);
    const wfValidation = sdk.validateWorkflow(workflow, context);
    if (!wfValidation.valid) {
      process.stdout.write(`${JSON.stringify({ kind: 'workflow_errors', issues: wfValidation.issues }, null, 2)}\n`);
      (globalThis as any).process.exitCode = 1;
      return;
    }
    const plan = sdk.buildWorkflowExecutionPlan(workflow, context);
    if (parsed.dryRun) {
      process.stdout.write(await dryRunCompilePlan({ sdk, plan, ctx: context }));
    } else {
      if (!config) {
        process.stdout.write('Missing --config for execution mode\n');
        process.exitCode = 1;
        return;
      }
      if (parsed.broadcast) {
        const missing = missingSignerChains(plan, config);
        if (missing.length > 0) {
          process.stdout.write(`Missing signer config for broadcast on chains: ${missing.join(', ')}\n`);
          process.exitCode = 1;
          return;
        }
      }
      let baseExecutors: any[];
      try {
        baseExecutors = await createExecutorsFromConfig(sdk, config, { allow_broadcast: Boolean(parsed.broadcast) });
      } catch (e) {
        process.stdout.write(`Failed to create executors from config: ${(e as Error)?.message ?? String(e)}\n`);
        process.exitCode = 1;
        return;
      }
      const executors = baseExecutors.map(
        (ex) =>
          new StrictSuccessExecutor(
            new BroadcastGateExecutor(
              sdk,
              new ActionPreflightExecutor(
                sdk,
                new PolicyGateExecutor(sdk, ex as any, { yes: Boolean(parsed.yes) })
              ),
              Boolean(parsed.broadcast)
            )
          )
      );
      if (executors.length === 0) {
        process.stdout.write('No executors created from config (missing/empty rpc_url?)\n');
        process.exitCode = 1;
        return;
      }
      const detect = createRunnerDetectResolver({ sdk });
      const baseSolver = sdk.createSolver ? sdk.createSolver() : sdk.solver;
      const solver = wrapSolverWithCalculatedFields({ sdk, inner: baseSolver, detect });
      const checkpoint_store = parsed.checkpointPath ? new FileCheckpointStore(sdk, parsed.checkpointPath) : undefined;
      process.stdout.write(`${formatPlanSummary(plan)}`);
      for await (const ev of sdk.runPlan(plan, context, { solver, executors, detect, checkpoint_store, resume_from_checkpoint: Boolean(parsed.resume) })) {
        applyRunnerSideEffects(sdk, context, ev);
        process.stdout.write(`${formatEvent(ev)}\n`);
        if (ev.type === 'engine_paused') break;
        if (ev.type === 'error') break;
      }
      await destroyExecutors(executors);
    }
    return;
  }

  // run_query
  if (!parsed.chain) {
    process.stdout.write('Missing --chain for query mode\n');
    (globalThis as any).process.exitCode = 1;
    return;
  }
  const [skill, query] = splitRef(parsed.queryRef);
  if (!skill || !query) {
    process.stdout.write('Invalid --ref for query mode (expected protocol@ver/<queryId>)\n');
    (globalThis as any).process.exitCode = 1;
    return;
  }
  const args = JSON.parse(parsed.argsJson);
  const resolved = sdk.resolveQuery(context, `${skill}/${query}`);
  if (!resolved) {
    process.stdout.write('Query not found in workspace\n');
    process.exitCode = 1;
    return;
  }
  const coercedArgs = coerceArgsByParams(resolved.query.params, args);
  const node: any = { id: 'n1', type: 'query_ref', chain: parsed.chain, skill, query, args: toLitValueRefs(coercedArgs) };
  if (parsed.untilCel) node.until = { cel: parsed.untilCel };
  if (parsed.retryJson) node.retry = JSON.parse(parsed.retryJson);
  if (parsed.timeoutMs !== undefined) node.timeout_ms = parsed.timeoutMs;
  const workflow = synthWorkflow('runner-query', parsed.chain, [node]);
  const wfValidation = sdk.validateWorkflow(workflow, context);
  if (!wfValidation.valid) {
    process.stdout.write(`${JSON.stringify({ kind: 'workflow_errors', issues: wfValidation.issues }, null, 2)}\n`);
    (globalThis as any).process.exitCode = 1;
    return;
  }
  const plan = sdk.buildWorkflowExecutionPlan(workflow, context);
  if (parsed.dryRun) {
    process.stdout.write(await dryRunCompilePlan({ sdk, plan, ctx: context }));
  } else {
    if (!config) {
      process.stdout.write('Missing --config for execution mode\n');
      process.exitCode = 1;
      return;
    }
    if (parsed.broadcast) {
      const missing = missingSignerChains(plan, config);
      if (missing.length > 0) {
        process.stdout.write(`Missing signer config for broadcast on chains: ${missing.join(', ')}\n`);
        process.exitCode = 1;
        return;
      }
    }
    let baseExecutors: any[];
    try {
      baseExecutors = await createExecutorsFromConfig(sdk, config, { allow_broadcast: Boolean(parsed.broadcast) });
    } catch (e) {
      process.stdout.write(`Failed to create executors from config: ${(e as Error)?.message ?? String(e)}\n`);
      process.exitCode = 1;
      return;
    }
    const executors = baseExecutors.map(
      (ex) =>
        new StrictSuccessExecutor(
          new BroadcastGateExecutor(
            sdk,
            new ActionPreflightExecutor(
              sdk,
              new PolicyGateExecutor(sdk, ex as any, { yes: Boolean(parsed.yes) })
            ),
            Boolean(parsed.broadcast)
          )
        )
    );
    if (executors.length === 0) {
      process.stdout.write('No executors created from config (missing/empty rpc_url?)\n');
      process.exitCode = 1;
      return;
    }
    const detect = createRunnerDetectResolver({ sdk });
    const baseSolver = sdk.createSolver ? sdk.createSolver() : sdk.solver;
    const solver = wrapSolverWithCalculatedFields({ sdk, inner: baseSolver, detect });
    const checkpoint_store = parsed.checkpointPath ? new FileCheckpointStore(sdk, parsed.checkpointPath) : undefined;
    process.stdout.write(`${formatPlanSummary(plan)}`);
    for await (const ev of sdk.runPlan(plan, context, { solver, executors, detect, checkpoint_store, resume_from_checkpoint: Boolean(parsed.resume) })) {
      applyRunnerSideEffects(sdk, context, ev);
      process.stdout.write(`${formatEvent(ev)}\n`);
      if (ev.type === 'engine_paused') break;
      if (ev.type === 'error') break;
    }
    await destroyExecutors(executors);
  }
}

function splitRef(ref: string): [string | null, string | null] {
  const i = ref.indexOf('/');
  if (i <= 0 || i >= ref.length - 1) return [null, null];
  return [ref.slice(0, i), ref.slice(i + 1)];
}

function synthWorkflow(name: string, defaultChain: string, nodes: any[]): any {
  return {
    schema: 'ais-flow/0.0.2',
    meta: { name, version: '0.0.2' },
    default_chain: defaultChain,
    nodes,
    extensions: {},
  };
}

function toLitValueRefs(value: any): Record<string, any> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('args must be a JSON object');
  }
  const out: Record<string, any> = {};
  for (const [k, v] of Object.entries(value)) out[k] = { lit: v };
  return out;
}

function collectRelevantWorkspacePaths(
  sdk: any,
  docs: any,
  workflowPath: string,
  workflow: any
): Set<string> {
  const out = new Set<string>();
  out.add(workflowPath);

  const req = workflow?.requires_pack;
  if (req && typeof req === 'object') {
    const wantName = String(req.name ?? '');
    const wantVer = String(req.version ?? '');
    for (const p of docs.packs ?? []) {
      const meta = p?.document?.meta;
      const name = meta?.name ? String(meta.name) : '';
      const version = meta?.version ? String(meta.version) : '';
      if (name === wantName && version === wantVer) {
        out.add(String(p.path));
        break;
      }
    }
  }

  for (const n of workflow?.nodes ?? []) {
    const skill = n?.skill;
    if (typeof skill !== 'string') continue;
    const parsed = sdk.parseSkillRef(skill);
    const protocol = String(parsed?.protocol ?? '');
    const version = String(parsed?.version ?? '');
    if (!protocol || !version) continue;
    for (const pr of docs.protocols ?? []) {
      const meta = pr?.document?.meta;
      if (String(meta?.protocol ?? '') === protocol && String(meta?.version ?? '') === version) {
        out.add(String(pr.path));
        break;
      }
    }
  }

  return out;
}

function findRequiredPackDocument(workflow: any, docs: any): any | null {
  const req = workflow?.requires_pack;
  if (!req || typeof req !== 'object') return null;
  const wantName = String(req.name ?? '');
  const wantVer = String(req.version ?? '');
  if (!wantName || !wantVer) return null;

  for (const p of docs?.packs ?? []) {
    const doc = p?.document;
    const meta = doc?.meta;
    const name = meta?.name ? String(meta.name) : doc?.name ? String(doc.name) : '';
    const version = meta?.version ? String(meta.version) : doc?.version ? String(doc.version) : '';
    if (name === wantName && version === wantVer) return p;
  }
  return null;
}

function formatEvent(ev: any): string {
  const t = String(ev?.type ?? '');
  if (t === 'plan_ready') return 'event: plan_ready';
  if (t === 'node_ready') return `event: node_ready node=${ev.node?.id}`;
  if (t === 'node_blocked') {
    const missing = ev.readiness?.missing_refs?.join(',') ?? '';
    const detect = ev.readiness?.needs_detect ? ' needs_detect' : '';
    return `event: node_blocked node=${ev.node?.id} missing=[${missing}]${detect}`;
  }
  if (t === 'solver_applied') return `event: solver_applied node=${ev.node?.id} patches=${ev.patches?.length ?? 0}`;
  if (t === 'query_result') return `event: query_result node=${ev.node?.id}`;
  if (t === 'tx_sent') return `event: tx_sent node=${ev.node?.id} hash=${ev.tx_hash}`;
  if (t === 'tx_confirmed') return `event: tx_confirmed node=${ev.node?.id}`;
  if (t === 'need_user_confirm') return `event: need_user_confirm node=${ev.node?.id} reason=${String(ev.reason ?? '')}`;
  if (t === 'node_waiting') return `event: node_waiting node=${ev.node?.id} attempts=${ev.attempts}`;
  if (t === 'skipped') return `event: skipped node=${ev.node?.id}`;
  if (t === 'engine_paused') return `event: engine_paused paused=${ev.paused?.length ?? 0}`;
  if (t === 'error') return `event: error node=${ev.node?.id ?? 'global'} msg=${ev.error?.message ?? String(ev.error)}`;
  if (t === 'checkpoint_saved') return `event: checkpoint_saved`;
  return `event: ${t}`;
}

function missingSignerChains(plan: any, config: any): string[] {
  const chainsWithWrites = new Set<string>();
  for (const node of plan?.nodes ?? []) {
    const t = String(node?.execution?.type ?? '');
    const isRead = t === 'evm_read' || t === 'evm_get_balance' || t === 'evm_multiread' || t === 'solana_read';
    if (!isRead) chainsWithWrites.add(String(node?.chain ?? ''));
  }
  const missing: string[] = [];
  for (const ch of chainsWithWrites) {
    if (!ch) continue;
    const s = config?.chains?.[ch]?.signer;
    if (!s) missing.push(ch);
  }
  return missing;
}

function applyRunnerSideEffects(sdk: any, ctx: any, ev: any): void {
  if (!ev || typeof ev !== 'object') return;

  // RUN-011: fan-out workflow query results into the legacy/flat runtime.query bag
  // so protocol actions/calculated_fields that reference `query["..."]` can work.
  if (ev.type === 'query_result') {
    const queryId = ev?.node?.source?.query;
    if (typeof queryId === 'string' && queryId.length > 0) {
      sdk.setQueryResult(ctx, queryId, ev.outputs ?? {});
    }
  }
}

async function destroyExecutors(executors: any[]): Promise<void> {
  await Promise.allSettled(
    (executors ?? []).map(async (ex) => {
      try {
        await ex?.destroy?.();
      } catch {
        // Best-effort cleanup: ignore.
      }
    })
  );
}
