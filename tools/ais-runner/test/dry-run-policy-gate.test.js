import test from 'node:test';
import assert from 'node:assert/strict';

import { dryRunCompilePlan } from '../dist/dry-run.js';

test('dryRunCompilePlan emits policy gate input/result for write nodes', async () => {
  const sdk = {
    solver: {
      solve() {
        return { patches: [] };
      },
    },
    getNodeReadiness() {
      return { state: 'ready', resolved_params: {} };
    },
    applyRuntimePatches() {},
    compileEvmExecution() {
      return {
        chain: 'eip155:1',
        chainId: 1,
        to: '0x1111111111111111111111111111111111111111',
        value: 0n,
        data: '0xabcdef',
        abi: { name: 'swapExactIn' },
      };
    },
    compileWritePreview() {
      return { kind: 'evm_tx', chain: 'eip155:1', function_name: 'swapExactIn' };
    },
    extractPolicyGateInput() {
      return { chain: 'eip155:1', action_ref: 'demo@0.0.2/swap', spend_amount: '1000', slippage_bps: 50 };
    },
    enforcePolicyGate() {
      return { ok: false, kind: 'need_user_confirm', reason: 'policy approval required', details: { approval_reasons: ['risk level'] } };
    },
    explainPolicyGateResult(result) {
      return { status: result.kind, reason: result.reason, details: result.details };
    },
    resolveAction() {
      return { action: { risk_level: 4, risk_tags: ['swap'] } };
    },
  };

  const plan = {
    schema: 'ais-plan/0.0.2',
    nodes: [
      {
        id: 'n1',
        kind: 'action_ref',
        chain: 'eip155:1',
        execution: { type: 'evm_call' },
        source: { protocol: 'demo@0.0.2', action: 'swap' },
      },
    ],
  };
  const ctx = { runtime: { ctx: {} } };
  const pack = { schema: 'ais-pack/0.0.2', includes: [] };

  const output = await dryRunCompilePlan({ sdk, plan, ctx, pack });
  assert.match(output, /policy_gate_input=/);
  assert.match(output, /policy_gate_result=/);
  assert.match(output, /need_user_confirm/);
});
