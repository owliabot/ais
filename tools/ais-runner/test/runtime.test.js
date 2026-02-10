import test from 'node:test';
import assert from 'node:assert/strict';

import { coerceArgsByParams, coerceByType, coerceWorkflowInputs } from '../dist/runtime.js';

test('coerceByType coerces uint/int to bigint', () => {
  assert.equal(coerceByType('uint256', '1'), 1n);
  assert.equal(coerceByType('uint32', 50), 50n);
  assert.equal(coerceByType('int128', '-7'), -7n);
});

test('coerceByType keeps token_amount as string by default', () => {
  assert.equal(coerceByType('token_amount', 123), '123');
  assert.equal(coerceByType('token_amount', '1.25'), '1.25');
  assert.equal(coerceByType('token_amount', 10n), 10n);
});

test('coerceWorkflowInputs applies defaults and coercion by declared input type', () => {
  const declared = {
    slippage_bps: { type: 'uint32', default: '50' },
  };
  const out = coerceWorkflowInputs(declared, {});
  assert.equal(out.slippage_bps, 50n);
});

test('coerceArgsByParams coerces args and applies defaults', () => {
  const params = [
    { name: 'amount', type: 'uint256', default: '7' },
    { name: 'memo', type: 'string' },
  ];
  const out = coerceArgsByParams(params, { memo: 123 });
  assert.equal(out.amount, 7n);
  assert.equal(out.memo, '123');
});

