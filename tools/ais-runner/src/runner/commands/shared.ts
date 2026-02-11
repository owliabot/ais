import { dryRunCompilePlan } from '../../dry-run.js';
import type { RunnerConfig } from '../../config.js';
import { ensureCtxNow } from '../../runtime.js';
import type { RunnerContext, RunnerSdkModule, RunnerWorkflow, RunnerWorkspaceDocuments } from '../../types.js';
import { executePlan } from '../engine/execute-plan.js';

type ExecuteRequestFlags = {
  dryRun?: boolean;
  broadcast?: boolean;
  yes?: boolean;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
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
    process.stdout.write(`${JSON.stringify({ kind: 'workflow_errors', issues: wfValidation.issues }, null, 2)}\n`);
    process.exitCode = 1;
    return;
  }

  const plan = sdk.buildWorkflowExecutionPlan(workflow, context);
  beforeDryRunOrExecute?.(plan);
  if (flags.dryRun) {
    process.stdout.write(await dryRunCompilePlan({ sdk, plan, ctx: context }));
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
    },
  });
}
