import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, PolicyGateOptions, WrapperSdk } from './types.js';
import { classifyIo, packMeta, policyApprovalsSummary, uniqStrings } from './util.js';

export class PolicyGateExecutor implements RunnerDestroyableExecutor {
  private readonly approvedByActionKey = new Set<string>();

  constructor(
    private readonly sdk: WrapperSdk,
    private readonly inner: RunnerDestroyableExecutor,
    private readonly opts: PolicyGateOptions
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
    const pack = this.opts.pack;
    const policy = pack?.policy;
    if (!policy || classifyIo(node) !== 'write') return await this.inner.execute(node, ctx, options);

    const protocolRef = node.source?.protocol;
    const actionId = node.source?.action;
    const workflowNodeId = String(node.source?.node_id ?? '');
    if (typeof protocolRef !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
      return await this.inner.execute(node, ctx, options);
    }

    const parsed = this.sdk.parseProtocolRef(protocolRef);
    const protocol = parsed.protocol ? String(parsed.protocol) : '';
    const version = parsed.version ? String(parsed.version) : '';

    const actionKey = protocol ? `${protocol}.${actionId}` : `${protocolRef}/${actionId}`;
    const gateKey = workflowNodeId ? `${workflowNodeId}:${actionKey}` : actionKey;
    if (this.approvedByActionKey.has(gateKey)) return await this.inner.execute(node, ctx, options);

    const resolved = this.sdk.resolveAction(ctx, `${protocolRef}/${actionId}`);
    if (!resolved) {
      return {
        need_user_confirm: {
          reason: 'action not found for policy gate (resolveAction failed)',
          details: { protocol: protocolRef, action: actionId, node_id: node.id },
        },
      };
    }

    const baseRiskLevel = resolved.action?.risk_level;
    const risk_level = typeof baseRiskLevel === 'number' ? baseRiskLevel : 3;

    const tagsFromAction = Array.isArray(resolved.action?.risk_tags) ? resolved.action.risk_tags : [];
    const overrides = pack?.overrides?.actions;
    const override =
      overrides && typeof overrides === 'object'
        ? (overrides[actionKey] ?? (protocol && version ? overrides[`${protocol}@${version}.${actionId}`] : undefined))
        : undefined;
    const tagsFromOverride = Array.isArray(override?.risk_tags) ? override.risk_tags : [];
    const risk_tags = uniqStrings([...tagsFromAction, ...tagsFromOverride]);

    const tokenPolicy = pack?.token_policy;
    const check = this.sdk.validateConstraints
      ? this.sdk.validateConstraints(policy, tokenPolicy, { chain: node.chain, risk_level, risk_tags })
      : { requires_approval: false, approval_reasons: [] };

    if (check?.requires_approval) {
      if (!this.opts.yes) {
        return {
          need_user_confirm: {
            reason: 'policy approval required',
            details: {
              node_id: node.id,
              workflow_node_id: workflowNodeId || undefined,
              step_id: node.source?.step_id,
              action_ref: `${protocolRef}/${actionId}`,
              action_key: actionKey,
              risk_level,
              risk_tags,
              approval_reasons: check.approval_reasons ?? [],
              pack: packMeta(pack),
              policy: policyApprovalsSummary(policy),
            },
          },
        };
      }
      this.approvedByActionKey.add(gateKey);
    }

    return await this.inner.execute(node, ctx, options);
  }
}
