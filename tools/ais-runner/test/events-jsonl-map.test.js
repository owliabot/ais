import test from 'node:test';
import assert from 'node:assert/strict';

import { normalizeEventForJsonl } from '../dist/runner/engine/event-jsonl-map.js';

test('normalizeEventForJsonl enriches need_user_confirm details with stable fields', () => {
  const event = {
    type: 'need_user_confirm',
    node: {
      id: 'n1',
      chain: 'eip155:1',
      execution: { type: 'evm_call' },
      source: { protocol: 'demo@0.0.2', action: 'swap', node_id: 'wf1' },
    },
    reason: 'policy approval required',
    details: {
      pack: { name: 'safe-defi-pack', version: '0.0.2' },
      policy: { mode: 'strict', strict: true },
      risk: { risk_level: 4, require_approval_min_risk_level: 3 },
      gate: {
        reason: 'policy approval required',
        details: { approval_reasons: ['risk too high'] },
      },
    },
  };

  const mapped = normalizeEventForJsonl(event);
  assert.equal(mapped.type, 'need_user_confirm');
  assert.equal(mapped.details.node_id, 'n1');
  assert.equal(mapped.details.workflow_node_id, 'wf1');
  assert.equal(mapped.details.action_ref, 'demo@0.0.2/swap');
  assert.equal(mapped.details.chain, 'eip155:1');
  assert.equal(mapped.details.execution_type, 'evm_call');
  assert.equal(mapped.details.kind, 'policy_gate');
  assert.deepEqual(mapped.details.hit_reasons, ['policy approval required', 'risk too high']);
  assert.deepEqual(mapped.details.pack_summary, { name: 'safe-defi-pack', version: '0.0.2' });
  assert.deepEqual(mapped.details.policy_summary, {
    mode: 'strict',
    strict: true,
    risk_level: 4,
    require_approval_min_risk_level: 3,
  });
});
