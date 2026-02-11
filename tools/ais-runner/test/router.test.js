import test from 'node:test';
import assert from 'node:assert/strict';

import { ChainBoundExecutor } from '../dist/runner/executors/chain-bound.js';

test('ChainBoundExecutor supports only exact matching chain', () => {
  const inner = { supports: () => true, execute: () => ({ outputs: { ok: true } }) };
  const ex = new ChainBoundExecutor('eip155:1', inner);

  assert.equal(ex.supports({ chain: 'eip155:1' }), true);
  assert.equal(ex.supports({ chain: 'eip155:8453' }), false);
  assert.equal(ex.supports({}), false);
});

test('ChainBoundExecutor respects inner supports()', () => {
  const inner = { supports: () => false, execute: () => ({ outputs: { ok: true } }) };
  const ex = new ChainBoundExecutor('eip155:1', inner);
  assert.equal(ex.supports({ chain: 'eip155:1' }), false);
});
