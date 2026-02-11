import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPatch,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, WrapperSdk } from './types.js';
import { topoOrderCalculatedFields } from './util.js';

export class CalculatedFieldsExecutor implements RunnerDestroyableExecutor {
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
    const protocolRef = node.source?.protocol;
    const actionId = node.source?.action;
    if (typeof protocolRef !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
      return await this.inner.execute(node, ctx, options);
    }

    const resolved = this.sdk.resolveAction(ctx, `${protocolRef}/${actionId}`);
    if (!resolved) {
      return {
        need_user_confirm: {
          reason: 'action not found for calculated_fields (resolveAction failed)',
          details: { protocol: protocolRef, action: actionId, node_id: node.id },
        },
      };
    }

    const calculated = resolved.action?.calculated_fields;
    if (!calculated || typeof calculated !== 'object') {
      return await this.inner.execute(node, ctx, options);
    }

    const order = topoOrderCalculatedFields(calculated);
    const computed: Record<string, unknown> = {};

    for (const name of order) {
      const def = calculated[name];
      const expr = def?.expr;
      if (!expr) continue;

      try {
        const resolvedParams = options?.resolved_params ?? {};
        const detect = options?.detect;
        const evalOpts =
          detect || resolvedParams
            ? { root_overrides: { params: resolvedParams }, detect }
            : undefined;
        const value = detect
          ? await this.sdk.evaluateValueRefAsync(expr, ctx, evalOpts)
          : this.sdk.evaluateValueRef(expr, ctx, evalOpts);
        computed[name] = value;
      } catch (error) {
        const msg = (error as Error)?.message ?? String(error);
        const needsDetect =
          msg.includes('Detect kind') || msg.includes('Async detect') || msg.includes('Detect provider');
        return {
          need_user_confirm: {
            reason: needsDetect ? 'calculated_fields requires detect resolution' : 'calculated_fields evaluation failed',
            details: {
              node_id: node.id,
              action_ref: `${protocolRef}/${actionId}`,
              field: name,
              error: msg,
            },
          },
        };
      }
    }

    const patches: RunnerPatch[] = [
      { op: 'merge', path: 'calculated', value: computed },
      { op: 'merge', path: `nodes.${node.id}.calculated`, value: computed },
    ];
    this.sdk.applyRuntimePatches(ctx, patches);

    return await this.inner.execute(node, ctx, options);
  }
}
