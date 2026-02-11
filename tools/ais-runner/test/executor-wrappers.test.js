import test from 'node:test';
import assert from 'node:assert/strict';

import { BroadcastGateExecutor } from '../dist/runner/executors/wrappers/broadcast-gate.js';
import { CalculatedFieldsExecutor } from '../dist/runner/executors/wrappers/calculated-fields.js';
import { PolicyGateExecutor } from '../dist/runner/executors/wrappers/policy-gate.js';

test('BroadcastGateExecutor returns need_user_confirm with compiled write preview details', async () => {
  const sdk = {
    compileEvmExecution: () => ({
      chain: 'eip155:1',
      chainId: 1,
      to: '0x1111111111111111111111111111111111111111',
      data: '0xabcdef',
      value: 123n,
      abi: { name: 'swapExactIn' },
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
  assert.deepEqual(result.need_user_confirm.details, {
    chain: 'eip155:1',
    chainId: 1,
    to: '0x1111111111111111111111111111111111111111',
    data: '0xabcdef',
    value: '123',
    abi: 'swapExactIn',
  });
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
  let validateCalls = 0;
  let innerCalls = 0;
  const sdk = {
    parseProtocolRef: () => ({ protocol: 'demo', version: '0.0.2' }),
    resolveAction: () => {
      resolveActionCalls++;
      return { action: { risk_level: 5, risk_tags: ['swap'] } };
    },
    validateConstraints: () => {
      validateCalls++;
      return { requires_approval: true, approval_reasons: ['risk level'] };
    },
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

  assert.equal(validateCalls, 2);
  assert.equal(resolveActionCalls, 2);
  assert.equal(innerCalls, 3);
});
