import test from 'node:test';
import assert from 'node:assert/strict';

import { parseCliArgs, renderHelp } from '../dist/cli.js';

test('cli parses --trace-redact for plan mode', () => {
  const parsed = parseCliArgs([
    'run',
    'plan',
    '--file',
    'fixtures/example.ais-plan.json',
    '--workspace',
    '.',
    '--trace-redact',
    'audit',
  ]);
  assert.equal(parsed.kind, 'run_plan');
  assert.equal(parsed.traceRedactMode, 'audit');
});

test('cli parses --commands-stdin-jsonl flag', () => {
  const parsed = parseCliArgs([
    'run',
    'plan',
    '--file',
    'fixtures/example.ais-plan.json',
    '--workspace',
    '.',
    '--commands-stdin-jsonl',
  ]);
  assert.equal(parsed.kind, 'run_plan');
  assert.equal(parsed.commandsStdinJsonl, true);
});

test('help includes --trace-redact option', () => {
  const help = renderHelp();
  assert.match(help, /--trace-redact <mode>/);
});
