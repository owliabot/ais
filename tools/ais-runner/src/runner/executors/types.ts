import type { RunnerSdkModule } from '../../types.js';

export type ExecutorSdk = Pick<RunnerSdkModule, 'EvmJsonRpcExecutor' | 'SolanaRpcExecutor'>;
export type EvmExecutorOptions = ConstructorParameters<RunnerSdkModule['EvmJsonRpcExecutor']>[0];
export type SolanaExecutorOptions = ConstructorParameters<RunnerSdkModule['SolanaRpcExecutor']>[0];
export type RunnerJsonRpcTransport = EvmExecutorOptions['transport'];
export type RunnerEvmSigner = NonNullable<EvmExecutorOptions['signer']>;
export type RunnerEvmTxRequest = Parameters<RunnerEvmSigner['signTransaction']>[0];
export type RunnerSolanaSigner = NonNullable<SolanaExecutorOptions['signer']>;
export type RunnerSolanaConnection = SolanaExecutorOptions['connection'];
export type SolanaSignTxInput = Parameters<RunnerSolanaSigner['signTransaction']>[0];

export type RunnerChainConfig = {
  rpc_url?: string;
  wait_for_receipt?: boolean;
  receipt_poll?: { interval_ms?: number; max_attempts?: number };
  commitment?: string;
  wait_for_confirmation?: boolean;
  send_options?: { skipPreflight?: boolean; maxRetries?: number; preflightCommitment?: string };
  signer?: Record<string, unknown>;
};

export type RunnerExecutorsConfig = {
  chains?: Record<string, RunnerChainConfig>;
};
