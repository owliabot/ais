import { coerceArgsByParams } from '../../runtime.js';
import type { RunQueryRequest } from '../../cli.js';
import type { RunnerConfig } from '../../config.js';
import type { RunnerSdkModule, RunnerWorkflowNode } from '../../types.js';
import { parseJsonObject } from '../io/json.js';
import { findProtocolPathByRef } from '../workspace/resolve.js';
import { splitRef, synthWorkflow, toLitValueRefs } from '../workflow/synth.js';
import { applyRunnerRuntimeCtx, runPreparedWorkflow } from './shared.js';

export async function runQueryCommand(args: {
  parsed: RunQueryRequest;
  config: RunnerConfig | null;
  sdk: RunnerSdkModule;
}): Promise<void> {
  const { parsed, config, sdk } = args;
  const { context, result } = await sdk.loadDirectoryAsContext(parsed.workspaceDir, { recursive: true });

  applyRunnerRuntimeCtx(context, config);

  if (!parsed.chain) {
    process.stdout.write('Missing --chain for query mode\n');
    process.exitCode = 1;
    return;
  }
  const [protocolRef, query] = splitRef(parsed.queryRef);
  if (!protocolRef || !query) {
    process.stdout.write('Invalid --ref for query mode (expected protocol@ver/<queryId>)\n');
    process.exitCode = 1;
    return;
  }
  const rawArgs = parseJsonObject(parsed.argsJson, '--args', sdk.parseAisJson);
  const resolved = sdk.resolveQuery(context, `${protocolRef}/${query}`);
  if (!resolved) {
    process.stdout.write('Query not found in workspace\n');
    process.exitCode = 1;
    return;
  }
  const coercedArgs = coerceArgsByParams(resolved.query.params, rawArgs);
  const importPath = findProtocolPathByRef(sdk, result, protocolRef);
  if (!importPath) {
    process.stdout.write(`Protocol not found in workspace for --ref=${protocolRef}\n`);
    process.exitCode = 1;
    return;
  }
  const node: RunnerWorkflowNode = {
    id: 'n1',
    type: 'query_ref',
    chain: parsed.chain,
    protocol: protocolRef,
    query,
    args: toLitValueRefs(coercedArgs),
  };
  if (parsed.untilCel) node.until = { cel: parsed.untilCel };
  if (parsed.retryJson) node.retry = parseJsonObject(parsed.retryJson, '--retry', sdk.parseAisJson) as RunnerWorkflowNode['retry'];
  if (parsed.timeoutMs !== undefined) node.timeout_ms = parsed.timeoutMs;
  const workflow = synthWorkflow('runner-query', parsed.chain, [node], { protocols: [{ protocol: protocolRef, path: importPath }] });
  await runPreparedWorkflow({
    sdk,
    config,
    context,
    workflow,
    strictImports: parsed.strictImports,
    flags: {
      dryRun: parsed.dryRun,
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
