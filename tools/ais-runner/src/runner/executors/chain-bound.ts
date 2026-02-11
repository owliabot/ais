import type { RunnerContext, RunnerDestroyableExecutor, RunnerPlanNode } from '../../types.js';

type ExecuteOptions = Parameters<RunnerDestroyableExecutor['execute']>[2];
type ExecuteResult = Awaited<ReturnType<RunnerDestroyableExecutor['execute']>>;

export class ChainBoundExecutor implements RunnerDestroyableExecutor {
  constructor(
    private readonly chain: string,
    private readonly inner: RunnerDestroyableExecutor,
    private readonly cleanup?: () => void | Promise<void>
  ) {}

  supports(node: RunnerPlanNode): boolean {
    return node.chain === this.chain && this.inner.supports(node);
  }

  async execute(
    node: RunnerPlanNode,
    ctx: RunnerContext,
    options?: ExecuteOptions
  ): Promise<ExecuteResult> {
    return await this.inner.execute(node, ctx, options);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
    await this.cleanup?.();
  }
}
