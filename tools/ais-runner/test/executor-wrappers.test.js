import test from 'node:test';
import assert from 'node:assert/strict';

import { BroadcastGateExecutor } from '../dist/runner/executors/wrappers/broadcast-gate.js';
import { CalculatedFieldsExecutor } from '../dist/runner/executors/wrappers/calculated-fields.js';
import { PolicyGateExecutor } from '../dist/runner/executors/wrappers/policy-gate.js';

test('BroadcastGateExecutor returns need_user_confirm with compiled write preview details', async () => {
  const sdk = {
    compileWritePreview: () => ({
      kind: 'evm_tx',
      chain: 'eip155:1',
      to: '0x1111111111111111111111111111111111111111',
      data: '0xabcdef',
    }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      throw new Error('inner executor should not be called when broadcast is disabled');
    },
  };
  const ex = new BroadcastGateExecutor(sdk, inner, false);

  const node = {
    id: 'p1',
    kind: 'action_ref',
    chain: 'eip155:1',
    execution: { type: 'evm_call' },
    source: { protocol: 'demo@0.0.2', action: 'swap' },
  };
  const result = await ex.execute(node, { runtime: {} }, { resolved_params: { amount_in: 1n } });

  assert.ok(result.need_user_confirm);
  assert.match(result.need_user_confirm.reason, /broadcast disabled/);
  assert.equal(result.need_user_confirm.details.kind, 'broadcast_gate');
  assert.equal(result.need_user_confirm.details.node_id, 'p1');
  assert.equal(result.need_user_confirm.details.action_ref, 'demo@0.0.2/swap');
  assert.equal(result.need_user_confirm.details.chain, 'eip155:1');
  assert.equal(result.need_user_confirm.details.execution_type, 'evm_call');
  assert.deepEqual(result.need_user_confirm.details.hit_reasons, ['broadcast_disabled']);
  assert.equal(result.need_user_confirm.details.preview.kind, 'evm_tx');
});

test('CalculatedFieldsExecutor evaluates calculated fields in dependency order', async () => {
  const evaluated = [];
  const sdk = {
    resolveAction: () => ({
      action: {
        calculated_fields: {
          b: { inputs: ['calculated.a'], expr: { ref: 'expr.b' } },
          a: { inputs: [], expr: { ref: 'expr.a' } },
        },
      },
    }),
    evaluateValueRef(expr) {
      const name = String(expr.ref).split('.').at(-1);
      evaluated.push(name);
      return `value-${name}`;
    },
    evaluateValueRefAsync: async () => {
      throw new Error('evaluateValueRefAsync should not be used without detect');
    },
    applyRuntimePatches(ctx, patches) {
      ctx.patches = patches;
    },
  };
  const inner = {
    supports: () => true,
    async execute() {
      return { outputs: { ok: true } };
    },
  };
  const ex = new CalculatedFieldsExecutor(sdk, inner);

  const node = {
    id: 'n1',
    kind: 'action_ref',
    chain: 'eip155:1',
    execution: { type: 'evm_call' },
    source: { protocol: 'demo@0.0.2', action: 'swap' },
  };
  const ctx = { runtime: {} };
  const result = await ex.execute(node, ctx, { resolved_params: {} });

  assert.deepEqual(evaluated, ['a', 'b']);
  assert.deepEqual(ctx.patches, [
    { op: 'merge', path: 'calculated', value: { a: 'value-a', b: 'value-b' } },
    { op: 'merge', path: 'nodes.n1.calculated', value: { a: 'value-a', b: 'value-b' } },
  ]);
  assert.deepEqual(result.outputs, { ok: true });
});

test('PolicyGateExecutor memoizes approvals by workflow node id + action key', async () => {
  let resolveActionCalls = 0;
  let gateCalls = 0;
  let innerCalls = 0;
  const sdk = {
    parseProtocolRef: () => ({ protocol: 'demo', version: '0.0.2' }),
    checkExecutionPluginAllowed: () => ({ ok: true }),
    resolveAction: () => {
      resolveActionCalls++;
      return { action: { risk_level: 5, risk_tags: ['swap'] } };
    },
    compileWritePreview: () => ({ kind: 'evm_tx', function_name: 'swapExactIn' }),
    extractPolicyGateInput: () => ({ chain: 'eip155:1' }),
    enforcePolicyGate: () => {
      gateCalls++;
      return {
        ok: false,
        kind: 'need_user_confirm',
        reason: 'policy approval required',
        details: {
          approval_reasons: ['risk level'],
        },
      };
    },
    explainPolicyGateResult: (gate) => ({ status: gate.kind, reason: gate.reason, details: gate.details }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      innerCalls++;
      return { outputs: { ok: true } };
    },
  };
  const ex = new PolicyGateExecutor(sdk, inner, {
    yes: true,
    pack: { policy: { approvals: { require_approval_min_risk_level: 4 } } },
  });

  const base = {
    kind: 'action_ref',
    chain: 'eip155:1',
    execution: { type: 'evm_call' },
    source: { protocol: 'demo@0.0.2', action: 'swap' },
  };
  const nodeA = { ...base, id: 'p1', source: { ...base.source, node_id: 'w1' } };
  const nodeB = { ...base, id: 'p2', source: { ...base.source, node_id: 'w2' } };
  const ctx = { runtime: {} };

  await ex.execute(nodeA, ctx);
  await ex.execute(nodeB, ctx);
  await ex.execute(nodeA, ctx);

  assert.equal(gateCalls, 2);
  assert.equal(resolveActionCalls, 2);
  assert.equal(innerCalls, 3);
});

test('PolicyGateExecutor emits structured policy gate details', async () => {
  const sdk = {
    parseProtocolRef: () => ({ protocol: 'demo', version: '0.0.2' }),
    checkExecutionPluginAllowed: () => ({ ok: true }),
    resolveAction: () => ({ action: { risk_level: 5, risk_tags: ['swap'] } }),
    compileWritePreview: () => ({ kind: 'evm_tx', function_name: 'approve' }),
    extractPolicyGateInput: () => ({ chain: 'eip155:1' }),
    enforcePolicyGate: () => ({
      ok: false,
      kind: 'need_user_confirm',
      reason: 'policy approval required',
      details: {
        approval_reasons: ['risk too high'],
      },
    }),
    explainPolicyGateResult: (gate) => ({ status: gate.kind, reason: gate.reason, details: gate.details }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      return { outputs: { ok: true } };
    },
  };
  const ex = new PolicyGateExecutor(sdk, inner, {
    yes: false,
    pack: { meta: { name: 'demo-pack', version: '1.0.0' }, policy: { approvals: { require_approval_min_risk_level: 4 } } },
  });
  const node = {
    id: 'p1',
    kind: 'action_ref',
    chain: 'eip155:1',
    execution: { type: 'evm_call' },
    source: { protocol: 'demo@0.0.2', action: 'swap', node_id: 'wf1' },
  };

  const result = await ex.execute(node, { runtime: {} });
  assert.ok(result.need_user_confirm);
  assert.equal(result.need_user_confirm.details.kind, 'policy_gate');
  assert.equal(result.need_user_confirm.details.node_id, 'p1');
  assert.equal(result.need_user_confirm.details.workflow_node_id, 'wf1');
  assert.equal(result.need_user_confirm.details.action_ref, 'demo@0.0.2/swap');
  assert.equal(result.need_user_confirm.details.action_key, 'demo.swap');
  assert.equal(result.need_user_confirm.details.chain, 'eip155:1');
  assert.equal(result.need_user_confirm.details.execution_type, 'evm_call');
  assert.deepEqual(result.need_user_confirm.details.hit_reasons, ['policy approval required', 'risk too high']);
  assert.deepEqual(result.need_user_confirm.details.confirmation_scope, {
    mode: 'workflow_node',
    key: 'wf1',
    alternatives: ['action_key', 'tx_hash'],
  });
  assert.equal(result.need_user_confirm.details.confirmation_template.action.action_ref, 'demo@0.0.2/swap');
  assert.equal(result.need_user_confirm.details.confirmation_template.action.chain, 'eip155:1');
  assert.equal(result.need_user_confirm.details.confirmation_template.risk.level, 5);
  assert.equal(
    result.need_user_confirm.details.confirmation_template.risk.thresholds.require_approval_min_risk_level,
    4
  );
});

test('plugin allowlist fixture: allow plugin execution proceeds', async () => {
  let innerCalls = 0;
  const sdk = {
    checkExecutionPluginAllowed: () => ({ ok: true }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      innerCalls++;
      return { outputs: { ok: true } };
    },
  };
  const ex = new PolicyGateExecutor(sdk, inner, {
    yes: false,
    pack: undefined,
  });
  const node = {
    id: 'plugin1',
    kind: 'execution',
    chain: 'eip155:1',
    execution: { type: 'custom_plugin' },
    source: {},
  };

  const result = await ex.execute(node, { runtime: {} });
  assert.equal(innerCalls, 1);
  assert.deepEqual(result.outputs, { ok: true });
});

test('plugin allowlist fixture: deny plugin type returns need_user_confirm', async () => {
  let innerCalls = 0;
  const sdk = {
    checkExecutionPluginAllowed: () => ({
      ok: false,
      kind: 'hard_block',
      reason: 'plugin execution type is not allowlisted by pack',
      details: { type: 'custom_plugin' },
    }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      innerCalls++;
      return { outputs: { ok: true } };
    },
  };
  const ex = new PolicyGateExecutor(sdk, inner, {
    yes: false,
    pack: { meta: { name: 'pack-demo', version: '1.0.0' } },
  });
  const node = {
    id: 'plugin2',
    kind: 'execution',
    chain: 'eip155:1',
    execution: { type: 'custom_plugin' },
    source: {},
  };

  const result = await ex.execute(node, { runtime: {} });
  assert.equal(innerCalls, 0);
  assert.ok(result.need_user_confirm);
  assert.equal(result.need_user_confirm.details.kind, 'policy_allowlist');
  assert.equal(result.need_user_confirm.details.node_id, 'plugin2');
  assert.equal(result.need_user_confirm.details.chain, 'eip155:1');
  assert.equal(result.need_user_confirm.details.confirmation_scope.mode, 'workflow_node');
  assert.match(result.need_user_confirm.details.confirmation_template.summary, /allowlist/);
});

test('plugin allowlist fixture: chain dimension deny returns need_user_confirm', async () => {
  const sdk = {
    checkExecutionPluginAllowed: (_pack, input) => ({
      ok: false,
      kind: 'hard_block',
      reason: 'plugin execution type is not allowlisted by pack',
      details: { type: input.type, chain: input.chain },
    }),
  };
  const inner = {
    supports: () => true,
    async execute() {
      return { outputs: { ok: true } };
    },
  };
  const ex = new PolicyGateExecutor(sdk, inner, {
    yes: false,
    pack: { meta: { name: 'pack-demo', version: '1.0.0' } },
  });
  const node = {
    id: 'plugin3',
    kind: 'execution',
    chain: 'eip155:8453',
    execution: { type: 'custom_plugin' },
    source: {},
  };

  const result = await ex.execute(node, { runtime: {} });
  assert.ok(result.need_user_confirm);
  assert.equal(result.need_user_confirm.details.kind, 'policy_allowlist');
  assert.equal(result.need_user_confirm.details.chain, 'eip155:8453');
  assert.deepEqual(result.need_user_confirm.details.gate.details, {
    type: 'custom_plugin',
    chain: 'eip155:8453',
  });
});
