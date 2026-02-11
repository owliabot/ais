import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions } from './types.js';
import { asRecord, isEvmFailureStatus } from './util.js';

export class StrictSuccessExecutor implements RunnerDestroyableExecutor {
  constructor(private readonly inner: RunnerDestroyableExecutor) {}

  supports(node: RunnerPlanNode): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(
    node: RunnerPlanNode,
    ctx: RunnerContext,
    options?: ExecuteOptions
  ): Promise<RunnerExecutorResult> {
    const res = await this.inner.execute(node, ctx, options);
    const outputs = asRecord(res.outputs);
    if (!outputs) return res;

    const receipt = asRecord(outputs.receipt);
    if (receipt && 'status' in receipt && isEvmFailureStatus(receipt.status)) {
      throw new Error(`EVM receipt status indicates failure: status=${String(receipt.status)}`);
    }

    const confirmation = asRecord(outputs.confirmation);
    const confirmationValue = confirmation ? asRecord(confirmation.value) : null;
    const confirmationErr = confirmationValue?.err;
    if (confirmationErr !== undefined && confirmationErr !== null) {
      throw new Error(`Solana confirmation indicates failure: err=${JSON.stringify(confirmationErr)}`);
    }

    return res;
  }
}
