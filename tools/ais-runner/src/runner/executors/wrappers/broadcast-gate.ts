import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, WrapperSdk } from './types.js';
import { classifyIo, isRunnerNodeApproved } from './util.js';

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
    if (!this.allowBroadcast && classifyIo(node) === 'write' && !isRunnerNodeApproved(ctx, node)) {
      const resolvedParams = options?.resolved_params ?? {};
      const details = this.sdk.compileWritePreview({
        node,
        ctx,
        resolved_params: resolvedParams,
      });
      return {
        need_user_confirm: {
          reason: 'broadcast disabled (pass --broadcast to allow write execution)',
          details: {
            kind: 'broadcast_gate',
            node_id: node.id,
            workflow_node_id: String(node.source?.node_id ?? node.id),
            action_ref: buildActionRef(node),
            chain: node.chain,
            execution_type: node.execution.type,
            hit_reasons: ['broadcast_disabled'],
            preview: details,
          },
        },
      };
    }
    return await this.inner.execute(node, ctx, options);
  }
}

function buildActionRef(node: RunnerPlanNode): string | undefined {
  const protocol = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const action = typeof node.source?.action === 'string' ? node.source.action : '';
  if (!protocol || !action) return undefined;
  return `${protocol}/${action}`;
}
