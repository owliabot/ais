import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, WrapperSdk } from './types.js';
import { classifyIo } from './util.js';

export class ActionPreflightExecutor implements RunnerDestroyableExecutor {
  constructor(
    private readonly sdk: WrapperSdk,
    private readonly inner: RunnerDestroyableExecutor
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
    if (classifyIo(node) !== 'write') return await this.inner.execute(node, ctx, options);

    const protocolRef = node.source?.protocol;
    const actionId = node.source?.action;
    if (typeof protocolRef === 'string' && typeof actionId === 'string' && actionId.length > 0) {
      const resolved = this.sdk.resolveAction(ctx, `${protocolRef}/${actionId}`);
      if (!resolved) {
        return {
          need_user_confirm: {
            reason: 'action not found for preflight (resolveAction failed)',
            details: { protocol: protocolRef, action: actionId, node_id: node.id },
          },
        };
      }

      const req = resolved.action?.requires_queries;
      if (Array.isArray(req) && req.length > 0) {
        const missing = req.filter((queryName) => ctx.runtime.query?.[queryName] === undefined);
        if (missing.length > 0) {
          return {
            need_user_confirm: {
              reason: 'missing required queries for action',
              details: { node_id: node.id, action_ref: `${protocolRef}/${actionId}`, missing_queries: missing },
            },
          };
        }
      }
    }

    return await this.inner.execute(node, ctx, options);
  }
}
