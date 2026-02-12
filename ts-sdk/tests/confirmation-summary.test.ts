import { describe, expect, it } from 'vitest';
import { summarizeNeedUserConfirm, type ExecutionPlanNode } from '../src/index.js';

describe('AGT106 confirmation summary', () => {
  it('builds stable hash and key fields from policy gate-like details', () => {
    const node: ExecutionPlanNode = {
      id: 'n1',
      chain: 'eip155:1' as any,
      kind: 'action_ref' as any,
      execution: { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'swap', inputs: [], outputs: [] }, args: {} } as any,
      source: { protocol: 'demo@0.0.2', action: 'swap', node_id: 'wf1', extensions: {} } as any,
      extensions: {} as any,
    } as any;

    const details = {
      kind: 'policy_gate',
      hit_reasons: ['policy approval required', 'risk too high'],
      gate: {
        status: 'need_user_confirm',
        reason: 'policy approval required',
        details: {
          gate_input: {
            risk_level: 4,
            risk_tags: ['swap'],
            slippage_bps: 100,
            preview: { kind: 'evm_tx', to: '0x' + '22'.repeat(20), function_name: 'swap', value: '0' },
          },
          approval_reasons: ['risk too high'],
        },
      },
    };

    const s1 = summarizeNeedUserConfirm({ node, reason: 'policy approval required', details });
    const s2 = summarizeNeedUserConfirm({ node, reason: 'policy approval required', details });

    expect(s1.schema).toBe('ais-confirmation-summary/0.0.1');
    expect(s1.hash).toBe(s2.hash);
    expect(s1.node.node_id).toBe('n1');
    expect(s1.node.workflow_node_id).toBe('wf1');
    expect(s1.node.action_ref).toBe('demo@0.0.2/swap');
    expect(s1.risk?.risk_level).toBe(4);
    expect(s1.hit_reasons?.length).toBeGreaterThan(0);
  });
});

