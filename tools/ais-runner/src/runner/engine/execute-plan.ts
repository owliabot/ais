import { FileCheckpointStore } from '../../checkpoint-store.js';
import { formatPlanSummary } from '../../plan-print.js';
import { wrapSolverWithCalculatedFields } from '../../solver-wrappers.js';
import { createRunnerDetectResolver } from '../../detect.js';
import { evaluateWorkflowOutputs, stringifyWithBigInt, writeOutputsJson } from '../../output.js';
import type { RunnerConfig } from '../../config.js';
import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerPlan,
  RunnerRunPlanOptions,
  RunnerSdkModule,
  RunnerWorkflow,
  RunnerWorkspaceDocuments,
} from '../../types.js';
import { formatEvent } from './events.js';
import { destroyExecutors, missingSignerChains } from './execution-helpers.js';
import { applyRunnerSideEffects } from './side-effects.js';
import { findRequiredPackDocument } from '../workspace/resolve.js';
import { buildExecutors } from '../executors/build.js';

type ExecuteFlags = {
  broadcast?: boolean;
  yes?: boolean;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
};

type ExecutePlanArgs = {
  sdk: RunnerSdkModule;
  config: RunnerConfig | null;
  plan: RunnerPlan;
  context: RunnerContext;
  flags: ExecuteFlags;
  workflow?: RunnerWorkflow;
  workspaceDocs?: RunnerWorkspaceDocuments;
  writeOutputsPath?: string;
};

export async function executePlan(args: ExecutePlanArgs): Promise<void> {
  const { sdk, config, plan, context, flags, workflow, workspaceDocs, writeOutputsPath } = args;
  const broadcast = Boolean(flags.broadcast);

  if (!config) {
    process.stdout.write('Missing --config for execution mode\n');
    process.exitCode = 1;
    return;
  }
  if (broadcast) {
    const missing = missingSignerChains(plan, config);
    if (missing.length > 0) {
      process.stdout.write(`Missing signer config for broadcast on chains: ${missing.join(', ')}\n`);
      process.exitCode = 1;
      return;
    }
  }

  let executors: RunnerDestroyableExecutor[];
  try {
    executors = await buildExecutors({
      sdk,
      config,
      broadcast,
      yes: Boolean(flags.yes),
      pack: workflow && workspaceDocs ? findRequiredPackDocument(workflow, workspaceDocs)?.document : undefined,
    });
  } catch (error) {
    process.stdout.write(`Failed to create executors from config: ${(error as Error)?.message ?? String(error)}\n`);
    process.exitCode = 1;
    return;
  }
  if (executors.length === 0) {
    process.stdout.write('No executors created from config (missing/empty rpc_url?)\n');
    process.exitCode = 1;
    return;
  }

  const detect = createRunnerDetectResolver({ sdk, workflow, workspaceDocs });
  const baseSolver = sdk.createSolver ? sdk.createSolver() : sdk.solver;
  const solver = wrapSolverWithCalculatedFields({ sdk, inner: baseSolver, detect });
  const trace = flags.tracePath ? { sink: sdk.createJsonlTraceSink({ file_path: flags.tracePath }) } : undefined;
  const checkpoint_store = flags.checkpointPath ? new FileCheckpointStore(sdk, flags.checkpointPath) : undefined;
  const options: RunnerRunPlanOptions = {
    solver,
    executors,
    detect,
    max_concurrency: config.engine?.max_concurrency,
    per_chain: config.engine?.per_chain,
    trace,
    checkpoint_store,
    resume_from_checkpoint: Boolean(flags.resume),
  };

  process.stdout.write(formatPlanSummary(plan));
  let endedEarly = false;
  try {
    for await (const ev of sdk.runPlan(plan, context, options)) {
      applyRunnerSideEffects(sdk, context, ev);
      process.stdout.write(`${formatEvent(ev)}\n`);
      if (ev.type === 'engine_paused' || ev.type === 'error') {
        endedEarly = true;
        break;
      }
    }
  } finally {
    await destroyExecutors(executors);
  }

  if (!endedEarly && workflow) {
    const evaluated = evaluateWorkflowOutputs(sdk, workflow, context);
    const payload = {
      kind: 'workflow_outputs',
      outputs: evaluated.outputs,
      errors: evaluated.errors,
    };
    process.stdout.write(`${stringifyWithBigInt(payload)}\n`);
    if (writeOutputsPath) await writeOutputsJson(writeOutputsPath, payload);
  }
}
