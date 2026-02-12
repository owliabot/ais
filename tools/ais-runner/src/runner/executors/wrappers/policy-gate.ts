import type {
  RunnerContext,
  RunnerDestroyableExecutor,
  RunnerExecutorResult,
  RunnerPlanNode,
} from '../../../types.js';
import type { ExecuteOptions, PolicyGateOptions, WrapperSdk } from './types.js';
import { classifyIo, packMeta, policyApprovalsSummary, uniqStrings, isRunnerNodeApproved } from './util.js';

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
    const pluginCheck = this.sdk.checkExecutionPluginAllowed(pack, {
      type: node.execution.type,
      chain: node.chain,
    });
    if (!pluginCheck.ok) {
      return {
        need_user_confirm: {
          reason: pluginCheck.reason ?? 'plugin execution blocked by pack',
          details: buildNeedUserConfirmDetails({
            node,
            pack,
            kind: 'policy_allowlist',
            gate: pluginCheck,
          }),
        },
      };
    }

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

    const actionKey = protocol ? `${protocol}.${actionId}` : `${protocolRef}/${actionId}`;
    const gateKey = workflowNodeId ? `${workflowNodeId}:${actionKey}` : actionKey;
    if (this.approvedByActionKey.has(gateKey)) return await this.inner.execute(node, ctx, options);

    const resolved = this.sdk.resolveAction(ctx, `${protocolRef}/${actionId}`);
    if (!resolved) {
      return {
        need_user_confirm: {
          reason: 'action not found for policy gate (resolveAction failed)',
          details: buildNeedUserConfirmDetails({
            node,
            pack,
            kind: 'policy_gate_resolution',
            hit_reasons: ['resolve_action_failed'],
            gate: {
              status: 'need_user_confirm',
              reason: 'action not found for policy gate (resolveAction failed)',
              details: { protocol: protocolRef, action: actionId },
            },
          }),
        },
      };
    }

    const gateInput = this.sdk.extractPolicyGateInput({
      node,
      ctx,
      pack,
      resolved_params: options?.resolved_params,
      action_risk_level: resolved.action?.risk_level,
      action_risk_tags: resolved.action?.risk_tags,
      runtime_risk_level: asNumber((ctx.runtime.policy as Record<string, unknown> | undefined)?.runtime_risk_level),
      runtime_risk_tags: asStringArray((ctx.runtime.policy as Record<string, unknown> | undefined)?.runtime_risk_tags),
      detect_result: asRecord((ctx.runtime.ctx as Record<string, unknown> | undefined)?.detect_result) ?? undefined,
      preview: this.sdk.compileWritePreview({
        node,
        ctx,
        resolved_params: options?.resolved_params ?? {},
      }),
    });
    const gate = this.sdk.enforcePolicyGate(pack, gateInput);
    if (!gate.ok) {
      if (gate.kind === 'hard_block') {
        return {
          need_user_confirm: {
            reason: gate.reason ?? 'policy hard block',
            details: buildNeedUserConfirmDetails({
              node,
              pack,
              kind: 'policy_gate',
              risk_level: resolved.action?.risk_level,
              gate: this.sdk.explainPolicyGateResult(gate),
            }),
          },
        };
      }
      if (!this.opts.yes) {
        if (isRunnerNodeApproved(ctx, node)) {
          this.approvedByActionKey.add(gateKey);
          return await this.inner.execute(node, ctx, options);
        }
        return {
          need_user_confirm: {
            reason: gate.reason ?? 'policy approval required',
            details: buildNeedUserConfirmDetails({
              node,
              pack,
              kind: 'policy_gate',
              risk_level: resolved.action?.risk_level,
              gate: this.sdk.explainPolicyGateResult(gate),
            }),
          },
        };
      }
      this.approvedByActionKey.add(gateKey);
    }

    return await this.inner.execute(node, ctx, options);
  }
}

function buildNeedUserConfirmDetails(options: {
  node: RunnerPlanNode;
  pack: PolicyGateOptions['pack'];
  kind: string;
  risk_level?: number;
  gate?: unknown;
  hit_reasons?: string[];
}): Record<string, unknown> {
  const { node, pack } = options;
  const gate = asRecord(options.gate);
  const hitReasons = uniqStrings([
    ...(options.hit_reasons ?? []),
    ...extractHitReasonsFromGate(gate),
  ]);
  const scope = buildConfirmationScope(node);
  const template = buildConfirmationTemplate({
    node,
    pack,
    kind: options.kind,
    risk_level: options.risk_level,
    gate,
    hit_reasons: hitReasons,
    scope,
  });

  return {
    kind: options.kind,
    node_id: node.id,
    workflow_node_id: String(node.source?.node_id ?? node.id),
    action_ref: buildActionRef(node),
    action_key: buildActionKey(node),
    chain: node.chain,
    execution_type: node.execution.type,
    pack: packMeta(pack),
    policy: policyApprovalsSummary(pack?.policy),
    gate: options.gate,
    hit_reasons: hitReasons,
    confirmation_scope: scope,
    confirmation_template: template,
  };
}

function extractHitReasonsFromGate(gate: Record<string, unknown> | null): string[] {
  if (!gate) return [];
  const out: string[] = [];
  const push = (value: unknown) => {
    if (typeof value !== 'string' || value.length === 0) return;
    if (!out.includes(value)) out.push(value);
  };

  push(gate.reason);
  const details = asRecord(gate.details);
  if (!details) return out;
  for (const reason of asStringArray(details.approval_reasons)) push(reason);
  for (const field of asStringArray(details.missing_fields)) push(`missing_field:${field}`);
  const violations = Array.isArray(details.violations) ? details.violations : [];
  for (const violation of violations) {
    const rec = asRecord(violation);
    if (!rec) continue;
    push(rec.message);
    push(rec.constraint);
  }
  return out;
}

function buildActionRef(node: RunnerPlanNode): string | undefined {
  const protocol = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const action = typeof node.source?.action === 'string' ? node.source.action : '';
  if (!protocol || !action) return undefined;
  return `${protocol}/${action}`;
}

function buildActionKey(node: RunnerPlanNode): string | undefined {
  const protocol = typeof node.source?.protocol === 'string' ? node.source.protocol : '';
  const action = typeof node.source?.action === 'string' ? node.source.action : '';
  if (!protocol || !action) return undefined;
  const parsed = protocol.split('@', 1)[0] ?? protocol;
  return `${parsed}.${action}`;
}

function buildConfirmationScope(node: RunnerPlanNode): {
  mode: 'workflow_node';
  key: string;
  alternatives: Array<'action_key' | 'tx_hash'>;
} {
  const workflowNodeId = String(node.source?.node_id ?? node.id);
  return {
    mode: 'workflow_node',
    key: workflowNodeId,
    alternatives: ['action_key', 'tx_hash'],
  };
}

function buildConfirmationTemplate(options: {
  node: RunnerPlanNode;
  pack: PolicyGateOptions['pack'];
  kind: string;
  risk_level?: number;
  gate: Record<string, unknown> | null;
  hit_reasons: string[];
  scope: { mode: 'workflow_node'; key: string; alternatives: Array<'action_key' | 'tx_hash'> };
}): Record<string, unknown> {
  const { node, pack, kind, risk_level, gate, hit_reasons, scope } = options;
  const actionRef = buildActionRef(node);
  const actionKey = buildActionKey(node);
  const gateDetails = asRecord(gate?.details);
  const gateInput = asRecord(gateDetails?.gate_input);
  const thresholds = collectThresholds(pack);
  const riskLevel = typeof gateInput?.risk_level === 'number' ? gateInput.risk_level : risk_level;

  return {
    title: kind === 'policy_allowlist' ? '需要确认：执行类型不在 allowlist' : '需要确认：策略规则触发',
    summary: summarizePrompt(kind, hit_reasons),
    action: {
      action_ref: actionRef,
      action_key: actionKey,
      chain: node.chain,
      execution_type: node.execution.type,
      workflow_node_id: scope.key,
    },
    risk: {
      level: riskLevel,
      hit_rules: hit_reasons,
      thresholds,
    },
    recommendation: recommendAction(kind),
  };
}

function summarizePrompt(kind: string, hitReasons: string[]): string {
  if (kind === 'policy_allowlist') {
    return '当前节点触发 allowlist 规则，请确认是否继续。';
  }
  if (hitReasons.length > 0) {
    return `当前节点命中策略规则：${hitReasons.slice(0, 2).join('；')}`;
  }
  return '当前节点触发策略规则，请确认是否继续。';
}

function recommendAction(kind: string): string {
  if (kind === 'policy_allowlist') {
    return '切换到 pack 允许的 provider/execution type，或取消执行。';
  }
  return '核对风险与阈值后确认；若不符合预期请取消并调整参数。';
}

function collectThresholds(pack: PolicyGateOptions['pack']): Record<string, unknown> | undefined {
  const policy = pack?.policy;
  if (!policy) return undefined;
  const approvals = policy.approvals;
  const hc = policy.hard_constraints_defaults ?? policy.hard_constraints;
  const out: Record<string, unknown> = {};
  if (approvals?.auto_execute_max_risk_level !== undefined) {
    out.auto_execute_max_risk_level = approvals.auto_execute_max_risk_level;
  }
  if (approvals?.require_approval_min_risk_level !== undefined) {
    out.require_approval_min_risk_level = approvals.require_approval_min_risk_level;
  }
  if (hc?.max_slippage_bps !== undefined) out.max_slippage_bps = hc.max_slippage_bps;
  if (hc?.allow_unlimited_approval !== undefined) out.allow_unlimited_approval = hc.allow_unlimited_approval;
  return Object.keys(out).length > 0 ? out : undefined;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0);
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}
