import { coerceArgsByParams } from '../../runtime.js';
import type { RunActionRequest } from '../../cli.js';
import type { RunnerConfig } from '../../config.js';
import type { RunnerSdkModule } from '../../types.js';
import { parseJsonObject } from '../io/json.js';
import { findProtocolPathByRef } from '../workspace/resolve.js';
import { splitRef, synthWorkflow, toLitValueRefs } from '../workflow/synth.js';
import { applyRunnerRuntimeCtx, runPreparedWorkflow } from './shared.js';

export async function runActionCommand(args: {
  parsed: RunActionRequest;
  config: RunnerConfig | null;
  sdk: RunnerSdkModule;
}): Promise<void> {
  const { parsed, config, sdk } = args;
  const { context, result } = await sdk.loadDirectoryAsContext(parsed.workspaceDir, { recursive: true });

  applyRunnerRuntimeCtx(context, config);

  if (!parsed.chain) {
    process.stdout.write('Missing --chain for action mode\n');
    process.exitCode = 1;
    return;
  }
  const [protocolRef, action] = splitRef(parsed.actionRef);
  if (!protocolRef || !action) {
    process.stdout.write('Invalid --ref for action mode (expected protocol@ver/<actionId>)\n');
    process.exitCode = 1;
    return;
  }
  const rawArgs = parseJsonObject(parsed.argsJson, '--args');
  const resolved = sdk.resolveAction(context, `${protocolRef}/${action}`);
  if (!resolved) {
    process.stdout.write('Action not found in workspace\n');
    process.exitCode = 1;
    return;
  }
  const coercedArgs = coerceArgsByParams(resolved.action.params, rawArgs);
  const importPath = findProtocolPathByRef(sdk, result, protocolRef);
  if (!importPath) {
    process.stdout.write(`Protocol not found in workspace for --ref=${protocolRef}\n`);
    process.exitCode = 1;
    return;
  }
  const workflow = synthWorkflow(
    'runner-action',
    parsed.chain,
    [{ id: 'n1', type: 'action_ref', chain: parsed.chain, protocol: protocolRef, action, args: toLitValueRefs(coercedArgs) }],
    { protocols: [{ protocol: protocolRef, path: importPath }] }
  );
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
    },
  });
}
