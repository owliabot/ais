import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, WrapperSdk } from './types.js';
import { classifyIo } from './util.js';
import { compileWritePreview } from './write-preview.js';

export class BroadcastGateExecutor implements RunnerDestroyableExecutor {
  constructor(
    private readonly sdk: WrapperSdk,
    private readonly inner: RunnerDestroyableExecutor,
    private readonly allowBroadcast: boolean
  ) {}

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
    if (!this.allowBroadcast && classifyIo(node) === 'write') {
      const resolvedParams = options?.resolved_params ?? {};
      const details = compileWritePreview(this.sdk, node, ctx, resolvedParams);
      return {
        need_user_confirm: {
          reason: 'broadcast disabled (pass --broadcast to allow write execution)',
          details,
        },
      };
    }
    return await this.inner.execute(node, ctx, options);
  }
}
