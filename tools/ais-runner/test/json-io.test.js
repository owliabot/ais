import test from 'node:test';
import assert from 'node:assert/strict';

import { parseJsonObject } from '../dist/runner/io/json.js';

test('parseJsonObject supports AIS tagged bigint via injected parser', () => {
  const parsed = parseJsonObject(
    '{"amount":{"__ais_json_type":"bigint","value":"7"}}',
    '--inputs',
    (json) =>
      JSON.parse(json, (_k, v) => {
        if (v && v.__ais_json_type === 'bigint' && typeof v.value === 'string') return BigInt(v.value);
        return v;
      })
  );
  assert.equal(parsed.amount, 7n);
});

test('parseJsonObject rejects non-object input', () => {
  assert.throws(() => parseJsonObject('[]', '--ctx'), /--ctx must be a JSON object/);
});
