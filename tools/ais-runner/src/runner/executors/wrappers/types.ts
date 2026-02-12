import type { RunnerExecutor, RunnerPack, RunnerSdkModule } from '../../../types.js';

export type ExecuteOptions = Parameters<RunnerExecutor['execute']>[2];

export type WrapperSdk = Pick<
  RunnerSdkModule,
  | 'resolveAction'
  | 'parseProtocolRef'
  | 'validateConstraints'
  | 'extractPolicyGateInput'
  | 'compileWritePreview'
  | 'enforcePolicyGate'
  | 'explainPolicyGateResult'
  | 'checkExecutionPluginAllowed'
  | 'evaluateValueRef'
  | 'evaluateValueRefAsync'
  | 'applyRuntimePatches'
>;

export type PolicyGateOptions = {
  pack?: RunnerPack;
  yes?: boolean;
};
