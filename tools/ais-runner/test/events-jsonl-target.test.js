import test from 'node:test';
import assert from 'node:assert/strict';

import { resolveEventsJsonlTarget } from '../dist/runner/engine/execute-plan.js';

test('resolveEventsJsonlTarget supports stdout aliases', () => {
  const a = resolveEventsJsonlTarget('stdout');
  assert.equal(a.to_stdout, true);
  assert.equal(a.stream, process.stdout);

  const b = resolveEventsJsonlTarget('-');
  assert.equal(b.to_stdout, true);
  assert.equal(b.stream, process.stdout);
});

test('resolveEventsJsonlTarget keeps file paths unchanged', () => {
  const target = resolveEventsJsonlTarget('/tmp/engine.events.jsonl');
  assert.equal(target.to_stdout, false);
  assert.equal(target.file_path, '/tmp/engine.events.jsonl');
});
