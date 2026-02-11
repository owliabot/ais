import { createExecutorsFromConfig } from './factory.js';
import { ActionPreflightExecutor } from './wrappers/action-preflight.js';
import { BroadcastGateExecutor } from './wrappers/broadcast-gate.js';
import { PolicyGateExecutor } from './wrappers/policy-gate.js';
import { StrictSuccessExecutor } from './wrappers/strict-success.js';
import type { RunnerConfig } from '../../config.js';
import type { RunnerDestroyableExecutor, RunnerPack, RunnerSdkModule } from '../../types.js';

type BuildExecutorsArgs = {
  sdk: RunnerSdkModule;
  config: RunnerConfig;
  broadcast: boolean;
  yes: boolean;
  pack?: RunnerPack;
};

export async function buildExecutors(args: BuildExecutorsArgs): Promise<RunnerDestroyableExecutor[]> {
  const { sdk, config, broadcast, yes, pack } = args;
  const baseExecutors = await createExecutorsFromConfig(sdk, config, { allow_broadcast: broadcast });
  return baseExecutors.map(
    (executor) =>
      new StrictSuccessExecutor(
        new BroadcastGateExecutor(
          sdk,
          new ActionPreflightExecutor(
            sdk,
            new PolicyGateExecutor(sdk, executor, { pack, yes })
          ),
          broadcast
        )
      )
  );
}
