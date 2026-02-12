import test from 'node:test';
import assert from 'node:assert/strict';

import { diffPlans } from '../dist/plan-diff.js';

test('plan diff detects added/removed/changed nodes and key fields', () => {
  const a = {
    schema: 'ais-plan/0.0.3',
    nodes: [
      { id: 'n1', chain: 'eip155:1', kind: 'execution', deps: ['x'], execution: { type: 'evm_call', to: { lit: '0x1' } }, writes: [{ path: 'ctx.x', mode: 'set' }] },
      { id: 'n2', chain: 'eip155:1', kind: 'execution', execution: { type: 'evm_read' } },
    ],
    extensions: {},
  };
  const b = {
    schema: 'ais-plan/0.0.3',
    nodes: [
      { id: 'n1', chain: 'eip155:137', kind: 'execution', deps: [], execution: { type: 'evm_call', to: { lit: '0x2' } }, writes: [{ path: 'ctx.x', mode: 'merge' }] },
      { id: 'n3', chain: 'eip155:1', kind: 'execution', execution: { type: 'solana_instruction' } },
    ],
    extensions: {},
  };

  const d = diffPlans(a, b);
  assert.equal(d.kind, 'plan_diff');
  assert.deepEqual(d.added, ['n3']);
  assert.deepEqual(d.removed, ['n2']);
  assert.equal(d.changed.length, 1);
  assert.equal(d.changed[0].id, 'n1');
  const fields = d.changed[0].changes.map((c) => c.field).sort();
  assert.ok(fields.includes('chain'));
  assert.ok(fields.includes('deps'));
  assert.ok(fields.includes('writes'));
  assert.ok(fields.includes('execution'));
});

