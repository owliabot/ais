import type { RunnerDestroyableExecutor } from '../../types.js';
import { ChainBoundExecutor } from './chain-bound.js';
import { createEthersTransport, createEvmSignerFromConfig } from './evm/ethers-adapter.js';
import { createSolanaConnection, createSolanaSignerFromConfig, toCommitment, toSendOptions } from './solana/solana-adapter.js';
import type { ExecutorSdk, RunnerExecutorsConfig } from './types.js';

export async function createExecutorsFromConfig(
  sdk: ExecutorSdk,
  config: RunnerExecutorsConfig | null,
  options: { allow_broadcast: boolean }
): Promise<RunnerDestroyableExecutor[]> {
  const chains = config?.chains ?? {};
  const out: RunnerDestroyableExecutor[] = [];

  for (const [chain, chainConfig] of Object.entries(chains)) {
    const rpcUrl = String(chainConfig?.rpc_url ?? '').trim();
    if (!rpcUrl) continue;

    if (chain.startsWith('eip155:')) {
      const { provider, transport } = createEthersTransport(rpcUrl);
      const signer = options.allow_broadcast
        ? createEvmSignerFromConfig(chainConfig?.signer, provider)
        : undefined;
      const executor = new sdk.EvmJsonRpcExecutor({
        transport,
        signer,
        wait_for_receipt: Boolean(chainConfig?.wait_for_receipt),
        receipt_poll: {
          interval_ms: chainConfig?.receipt_poll?.interval_ms,
          max_attempts: chainConfig?.receipt_poll?.max_attempts,
        },
      });
      out.push(
        new ChainBoundExecutor(chain, executor, () => {
          provider.destroy?.();
        })
      );
      continue;
    }

    if (chain.startsWith('solana:')) {
      const connection = createSolanaConnection(rpcUrl, chainConfig?.commitment);
      const signer = options.allow_broadcast
        ? await createSolanaSignerFromConfig(chainConfig?.signer)
        : undefined;
      const executor = new sdk.SolanaRpcExecutor({
        connection,
        signer,
        commitment: toCommitment(chainConfig?.commitment),
        wait_for_confirmation: chainConfig?.wait_for_confirmation,
        send_options: toSendOptions(chainConfig?.send_options),
      });
      out.push(new ChainBoundExecutor(chain, executor));
      continue;
    }
  }

  return out;
}
