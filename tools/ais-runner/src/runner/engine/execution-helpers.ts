import type { RunnerConfig } from '../../config.js';
import type { RunnerDestroyableExecutor, RunnerPlan } from '../../types.js';

export function missingSignerChains(plan: RunnerPlan, config: RunnerConfig): string[] {
  const chainsWithWrites = new Set<string>();
  for (const node of plan.nodes) {
    const t = String(node.execution?.type ?? '');
    const isRead = t === 'evm_read' || t === 'evm_rpc' || t === 'evm_multiread' || t === 'solana_read';
    if (!isRead) chainsWithWrites.add(String(node.chain ?? ''));
  }
  const missing: string[] = [];
  for (const ch of chainsWithWrites) {
    if (!ch) continue;
    const signer = config.chains?.[ch]?.signer;
    if (!signer) missing.push(ch);
  }
  return missing;
}

export async function destroyExecutors(executors: RunnerDestroyableExecutor[]): Promise<void> {
  await Promise.allSettled(
    executors.map(async (executor) => {
      try {
        await executor.destroy?.();
      } catch {
        // Best-effort cleanup: ignore.
      }
    })
  );
}
