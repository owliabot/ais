import { describe, expect, it } from 'vitest';
import type { ExecutionPlanNode, ExecutionSpec, Pack } from '../src/index.js';
import { createContext, extractPolicyGateInput, enforcePolicyGate } from '../src/index.js';

function makeNode(execution: ExecutionSpec, chain = 'eip155:1', action = 'swap'): ExecutionPlanNode {
  return {
    id: `n_${action}`,
    chain,
    kind: 'action_ref',
    execution,
    source: {
      protocol: 'demo@0.0.2',
      action,
      node_id: `wf_${action}`,
    },
  };
}

describe('AGT010A gate input extraction rules', () => {
  it('risk_level priority: runtime > pack override > action', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: { type: 'function', name: 'approve', inputs: [{ name: 'spender', type: 'address' }, { name: 'amount', type: 'uint256' }], outputs: [] },
        args: { spender: { lit: '0x2222222222222222222222222222222222222222' }, amount: { lit: '1' } },
      },
      'eip155:1',
      'approve'
    );
    const pack = {
      schema: 'ais-pack/0.0.2',
      meta: { name: 'p', version: '1.0.0' },
      includes: [],
      overrides: {
        actions: {
          'demo.approve': { risk_level: 4 },
        },
      },
    } as Pack;

    const gate = extractPolicyGateInput({
      node,
      ctx,
      pack,
      resolved_params: {},
      action_risk_level: 2,
      runtime_risk_level: 5,
    });

    expect(gate.risk_level).toBe(5);
    expect(gate.field_sources?.risk_level).toContain('runtime');
  });

  it('spend/slippage/approval priority: params > calculated > detect_result', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x3333333333333333333333333333333333333333' },
        abi: {
          type: 'function',
          name: 'swapExactTokensForTokens',
          inputs: [{ name: 'amountIn', type: 'uint256' }, { name: 'slippageBps', type: 'uint256' }],
          outputs: [],
        },
        args: { amountIn: { lit: '1' }, slippageBps: { lit: '1' } },
      },
      'eip155:1',
      'swap'
    );
    const gate = extractPolicyGateInput({
      node,
      ctx,
      resolved_params: {
        spend_amount: '100',
        slippage_bps: 20,
        approval_amount: '7',
        calculated: { spend_amount: '200', slippage_bps: 30, approval_amount: '8' },
        detect_result: { spend_amount: '300', slippage_bps: 40, approval_amount: '9' },
      },
      detect_result: { spend_amount: '400', slippage_bps: 50, approval_amount: '10' },
    });
    expect(gate.spend_amount).toBe('100');
    expect(gate.slippage_bps).toBe(20);
    expect(gate.approval_amount).toBe('7');
    expect(gate.field_sources?.spend_amount).toContain('params');
    expect(gate.field_sources?.slippage_bps).toContain('params');
  });

  it('falls back to calculated then detect_result when params are absent', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x4444444444444444444444444444444444444444' },
        abi: { type: 'function', name: 'swapExactTokensForTokens', inputs: [{ name: 'amountIn', type: 'uint256' }], outputs: [] },
        args: { amountIn: { lit: '1' } },
      }
    );
    const gate = extractPolicyGateInput({
      node,
      ctx,
      resolved_params: {
        calculated: { spend_amount: '200', slippage_bps: 30 },
        detect_result: { spend_amount: '300', slippage_bps: 40 },
      },
    });
    expect(gate.spend_amount).toBe('200');
    expect(gate.slippage_bps).toBe(30);
    expect(gate.field_sources?.spend_amount).toContain('calculated');
  });

  it('hard-blocks when gate input has required hard_block_fields', () => {
    const result = enforcePolicyGate(
      { schema: 'ais-pack/0.0.2', meta: { name: 'p', version: '1.0.0' }, includes: [], policy: {} } as Pack,
      {
        chain: 'eip155:1',
        action_ref: 'demo@0.0.2/swap',
        hard_block_fields: ['preview_compile'],
      } as any
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).hard_block_fields).toEqual(['preview_compile']);
  });
});
