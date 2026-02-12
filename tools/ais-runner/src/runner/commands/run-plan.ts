import { readFile } from 'node:fs/promises';
import YAML from 'yaml';
import { dryRunCompilePlan, dryRunCompilePlanJson } from '../../dry-run.js';
import type { RunPlanRequest } from '../../cli.js';
import type { RunnerConfig } from '../../config.js';
import { ensureCtxNow } from '../../runtime.js';
import type {
  RunnerContext,
  RunnerPlan,
  RunnerSdkModule,
  RunnerWorkflow,
  RunnerWorkspaceDocuments,
  RunnerWorkspaceIssue,
} from '../../types.js';
import { parseJsonObject } from '../io/json.js';
import { executePlan } from '../engine/execute-plan.js';
import { collectRelevantWorkspacePaths, findRequiredPackDocument } from '../workspace/resolve.js';
import { structuredFromWorkspaceIssues, zodPathToFieldPath } from '../../issues.js';

export async function runPlanCommand(args: {
  parsed: RunPlanRequest;
  config: RunnerConfig | null;
  sdk: RunnerSdkModule;
}): Promise<void> {
  const { parsed, config, sdk } = args;
  const loaded = await sdk.loadDirectoryAsContext(parsed.workspaceDir, { recursive: true });
  const context = loaded.context;
  const workspaceDocs = loaded.result;
  const wsIssues = sdk.validateWorkspaceReferences(workspaceDocs);

  applyRunnerRuntimeCtx(context, config);
  if (parsed.inputsJson) {
    context.runtime.inputs = parseJsonObject(parsed.inputsJson, '--inputs', sdk.parseAisJson);
  }
  if (parsed.ctxJson) {
    const ctx = parseJsonObject(parsed.ctxJson, '--ctx', sdk.parseAisJson);
    context.runtime.ctx = { ...context.runtime.ctx, ...ctx };
    ensureCtxNow(context.runtime.ctx);
  }

  const workflow = parsed.workflowPath ? await sdk.loadWorkflow(parsed.workflowPath) : undefined;
  if (workflow) {
    const relevantPaths = collectRelevantWorkspacePaths(sdk, workspaceDocs, parsed.workflowPath!, workflow);
    const relevantIssues = wsIssues.filter((issue) => {
      const path = String(issue?.path ?? '');
      const relatedPath = issue?.related_path ? String(issue.related_path) : '';
      return relevantPaths.has(path) || (relatedPath && relevantPaths.has(relatedPath));
    });
    if (relevantIssues.some((issue) => issue?.severity === 'error')) {
      process.stdout.write(
        `${JSON.stringify({ kind: 'workspace_errors', issues: structuredFromWorkspaceIssues(relevantIssues) }, null, 2)}\n`
      );
      process.exitCode = 1;
      return;
    }
  }

  const parsedPlan = await loadAndValidatePlanFile(sdk, parsed.filePath);
  if (!parsedPlan.ok) {
    process.stdout.write(`${JSON.stringify(parsedPlan.error, null, 2)}\n`);
    process.exitCode = 1;
    return;
  }
  const plan = parsedPlan.plan;

  process.stdout.write(
    `${JSON.stringify(
      {
        request: parsed,
        config,
        load_errors: workspaceDocs.errors,
        workflow_path: parsed.workflowPath,
      },
      null,
      2
    )}\n\n`
  );

  if (parsed.dryRun) {
    const pack = workflow && workspaceDocs ? findRequiredPackDocument(workflow, workspaceDocs)?.document : undefined;
    const fmt = String((parsed as any).dryRunFormat ?? 'text');
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
    writeOutputsPath: parsed.outPath,
    flags: {
      broadcast: parsed.broadcast,
      yes: parsed.yes,
      checkpointPath: parsed.checkpointPath,
      resume: parsed.resume,
      tracePath: parsed.tracePath,
      traceRedactMode: parsed.traceRedactMode,
      eventsJsonlPath: parsed.eventsJsonlPath,
      commandsStdinJsonl: parsed.commandsStdinJsonl,
    },
  });
}

function applyRunnerRuntimeCtx(context: RunnerContext, config: RunnerConfig | null): void {
  if (config?.runtime?.ctx && typeof config.runtime.ctx === 'object') {
    context.runtime.ctx = { ...context.runtime.ctx, ...config.runtime.ctx };
  }
  ensureCtxNow(context.runtime.ctx);
}

export async function loadAndValidatePlanFile(
  sdk: RunnerSdkModule,
  filePath: string
): Promise<{ ok: true; plan: RunnerPlan } | { ok: false; error: unknown }> {
  let raw: string;
  try {
    raw = await readFile(filePath, 'utf-8');
  } catch (error) {
    return {
      ok: false,
      error: { kind: 'plan_load_error', path: filePath, message: (error as Error)?.message ?? String(error) },
    };
  }

  let doc: unknown;
  try {
    const isJson = filePath.endsWith('.json');
    doc = isJson ? sdk.parseAisJson(raw) : YAML.parse(raw);
  } catch (error) {
    return {
      ok: false,
      error: { kind: 'plan_parse_error', path: filePath, message: (error as Error)?.message ?? String(error) },
    };
  }

  const result = sdk.ExecutionPlanSchema.safeParse(doc);
  if (!result.success) {
    return {
      ok: false,
      error: {
        kind: 'plan_validation_error',
        path: filePath,
        issues: result.error.issues.map((issue) => ({
          kind: 'plan_validation',
          severity: 'error',
          field_path: zodPathToFieldPath(issue.path as any),
          message: issue.message,
          reference: issue.code,
        })),
      },
    };
  }

  return { ok: true, plan: result.data };
}
