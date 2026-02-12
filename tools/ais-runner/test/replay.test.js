import test from 'node:test';
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { replayCommand } from '../dist/runner/commands/replay.js';

function tmpFile(name) {
  return join(tmpdir(), `ais-runner-replay-${process.pid}-${Date.now()}-${name}`);
}

test('replay from checkpoint emits events until node', async () => {
  const checkpointPath = tmpFile('checkpoint.json');
  const checkpoint = {
    schema: 'ais-engine-checkpoint/0.0.2',
    created_at: '2020-01-01T00:00:00.000Z',
    plan: { schema: 'ais-plan/0.0.3', nodes: [], extensions: {} },
    runtime: {},
    completed_node_ids: [],
    events: [
      { type: 'plan_ready', plan: { schema: 'ais-plan/0.0.3', nodes: [], extensions: {} } },
      { type: 'node_ready', node: { id: 'n1', chain: 'eip155:1', kind: 'execution', execution: { type: 'evm_read' } } },
      { type: 'node_ready', node: { id: 'n2', chain: 'eip155:1', kind: 'execution', execution: { type: 'evm_read' } } },
    ],
  };
  await writeFile(checkpointPath, JSON.stringify(checkpoint), 'utf-8');

  const sdk = { deserializeCheckpoint: JSON.parse, parseAisJson: JSON.parse };

  let out = '';
  const prev = process.stdout.write;
  process.stdout.write = (chunk) => {
    out += String(chunk);
    return true;
  };
  try {
    await replayCommand({
      parsed: { kind: 'replay', checkpointPath, untilNodeId: 'n1', format: 'text' },
      sdk,
    });
  } finally {
    process.stdout.write = prev;
  }

  assert.ok(out.includes('source=checkpoint'));
  assert.ok(out.includes('node_ready node=n1'));
  assert.ok(!out.includes('node_ready node=n2'));
});

