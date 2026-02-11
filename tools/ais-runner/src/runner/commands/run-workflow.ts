import { coerceWorkflowInputs } from '../../runtime.js';
import type { RunWorkflowRequest } from '../../cli.js';
import type { RunnerConfig } from '../../config.js';
import type {
  RunnerContext,
  RunnerSdkModule,
  RunnerWorkflow,
  RunnerWorkspaceDocuments,
  RunnerWorkspaceIssue,
} from '../../types.js';
import { applyRunnerRuntimeCtx, runPreparedWorkflow } from './shared.js';
import { parseJsonObject } from '../io/json.js';
import { collectRelevantWorkspacePaths } from '../workspace/resolve.js';

export async function runWorkflowCommand(args: {
  parsed: RunWorkflowRequest;
  config: RunnerConfig | null;
  sdk: RunnerSdkModule;
}): Promise<void> {
  const { parsed, config, sdk } = args;
  let context: RunnerContext;
  let result: RunnerWorkspaceDocuments;
  let workflow: RunnerWorkflow;
  let wsIssues: RunnerWorkspaceIssue[] = [];

  if (parsed.importsOnly) {
    const bundle = await sdk.loadWorkflowBundle(parsed.filePath, {
      strict_imports: parsed.strictImports,
      search_paths: [parsed.workspaceDir],
    });
    context = bundle.context;
    workflow = bundle.workflow;
    result = {
      protocols: bundle.imports ?? [],
      packs: [],
      workflows: [{ path: parsed.filePath, document: workflow }],
      errors: [],
    };
  } else {
    const loaded = await sdk.loadDirectoryAsContext(parsed.workspaceDir, { recursive: true });
    context = loaded.context;
    result = loaded.result;
    wsIssues = sdk.validateWorkspaceReferences(result);
    workflow = await sdk.loadWorkflow(parsed.filePath);
  }

  applyRunnerRuntimeCtx(context, config);

  if (parsed.inputsJson) {
    context.runtime.inputs = parseJsonObject(parsed.inputsJson, '--inputs');
  }
  if (parsed.ctxJson) {
    const ctx = parseJsonObject(parsed.ctxJson, '--ctx');
    context.runtime.ctx = { ...context.runtime.ctx, ...ctx };
  }

  if (workflow?.inputs && typeof workflow.inputs === 'object') {
    context.runtime.inputs = coerceWorkflowInputs(workflow.inputs, context.runtime.inputs);
  }

  if (!parsed.importsOnly) {
    const relevantPaths = collectRelevantWorkspacePaths(sdk, result, parsed.filePath, workflow);
    const relevantIssues = wsIssues.filter((issue) => {
      const path = String(issue?.path ?? '');
      const relatedPath = issue?.related_path ? String(issue.related_path) : '';
      return relevantPaths.has(path) || (relatedPath && relevantPaths.has(relatedPath));
    });
    if (relevantIssues.some((issue) => issue?.severity === 'error')) {
      process.stdout.write(`${JSON.stringify({ kind: 'workspace_errors', issues: relevantIssues }, null, 2)}\n`);
      process.exitCode = 1;
      return;
    }
  }

  await runPreparedWorkflow({
    sdk,
    config,
    context,
    workflow,
    strictImports: parsed.strictImports,
    workspaceDocs: result,
    outPath: parsed.outPath,
    flags: {
      dryRun: parsed.dryRun,
      broadcast: parsed.broadcast,
      yes: parsed.yes,
      checkpointPath: parsed.checkpointPath,
      resume: parsed.resume,
      tracePath: parsed.tracePath,
    },
    beforeDryRunOrExecute: () => {
      process.stdout.write(`${JSON.stringify({ request: parsed, config, load_errors: result.errors }, null, 2)}\n\n`);
    },
  });
}
