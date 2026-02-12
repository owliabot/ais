import { dryRunCompilePlan, dryRunCompilePlanJson } from '../../dry-run.js';
import type { RunnerConfig } from '../../config.js';
import { ensureCtxNow } from '../../runtime.js';
import type { RunnerContext, RunnerSdkModule, RunnerWorkflow, RunnerWorkspaceDocuments } from '../../types.js';
import { executePlan } from '../engine/execute-plan.js';
import { findRequiredPackDocument } from '../workspace/resolve.js';
import { structuredFromWorkflowIssues } from '../../issues.js';

type ExecuteRequestFlags = {
  dryRun?: boolean;
  dryRunFormat?: string;
  broadcast?: boolean;
  yes?: boolean;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
  traceRedactMode?: string;
  eventsJsonlPath?: string;
  commandsStdinJsonl?: boolean;
};

export function applyRunnerRuntimeCtx(context: RunnerContext, config: RunnerConfig | null): void {
  if (config?.runtime?.ctx && typeof config.runtime.ctx === 'object') {
    context.runtime.ctx = { ...context.runtime.ctx, ...config.runtime.ctx };
  }
  ensureCtxNow(context.runtime.ctx);
}

export async function runPreparedWorkflow(args: {
  sdk: RunnerSdkModule;
  config: RunnerConfig | null;
  context: RunnerContext;
  workflow: RunnerWorkflow;
  strictImports?: boolean;
  workspaceDocs?: RunnerWorkspaceDocuments;
  outPath?: string;
  flags: ExecuteRequestFlags;
  beforeDryRunOrExecute?: (plan: ReturnType<RunnerSdkModule['buildWorkflowExecutionPlan']>) => void;
}): Promise<void> {
  const {
    sdk,
    config,
    context,
    workflow,
    strictImports,
    workspaceDocs,
    outPath,
    flags,
    beforeDryRunOrExecute,
  } = args;
  const wfValidation = sdk.validateWorkflow(workflow, context, { enforce_imports: strictImports });
  if (!wfValidation.valid) {
    process.stdout.write(
      `${JSON.stringify({ kind: 'workflow_errors', issues: structuredFromWorkflowIssues(wfValidation.issues) }, null, 2)}\n`
    );
    process.exitCode = 1;
    return;
  }

  let plan: ReturnType<RunnerSdkModule['buildWorkflowExecutionPlan']>;
  try {
    plan = sdk.buildWorkflowExecutionPlan(workflow, context);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    process.stdout.write(
      `${JSON.stringify(
        {
          kind: 'plan_build_errors',
          issues: [
            {
              kind: 'plan_build',
              severity: 'error',
              field_path: '(root)',
              message: msg,
            },
          ],
        },
        null,
        2
      )}\n`
    );
    process.exitCode = 1;
    return;
  }
  beforeDryRunOrExecute?.(plan);
  if (flags.dryRun) {
    const pack = workflow && workspaceDocs ? findRequiredPackDocument(workflow, workspaceDocs)?.document : undefined;
    const fmt = String((flags as any).dryRunFormat ?? 'text');
    if (fmt === 'json') {
      const payload = await dryRunCompilePlanJson({ sdk, plan, ctx: context, pack });
      process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
    } else {
      process.stdout.write(await dryRunCompilePlan({ sdk, plan, ctx: context, pack }));
    }
    return;
  }

  await executePlan({
    sdk,
    config,
    plan,
    context,
    workflow,
    workspaceDocs,
    writeOutputsPath: outPath,
    flags: {
      broadcast: flags.broadcast,
      yes: flags.yes,
      checkpointPath: flags.checkpointPath,
      resume: flags.resume,
      tracePath: flags.tracePath,
      traceRedactMode: flags.traceRedactMode,
      eventsJsonlPath: flags.eventsJsonlPath,
      commandsStdinJsonl: flags.commandsStdinJsonl,
    },
  });
}
