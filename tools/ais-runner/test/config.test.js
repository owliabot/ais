import test from 'node:test';
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { loadRunnerConfig } from '../dist/config.js';

function tmpFile(name) {
  return join(tmpdir(), `ais-runner-${process.pid}-${Date.now()}-${name}`);
}

test('loadRunnerConfig expands ${ENV} placeholders', async () => {
  const prev = process.env.MY_RPC;
  process.env.MY_RPC = 'http://example.invalid/rpc';
  try {
    const p = tmpFile('env.yaml');
    await writeFile(
      p,
      [
        'schema: "ais-runner/0.0.1"',
        'chains:',
        '  "eip155:1":',
        '    rpc_url: "${MY_RPC}"',
        '',
      ].join('\n'),
      'utf-8'
    );

    const cfg = await loadRunnerConfig(p);
    assert.equal(cfg.chains['eip155:1'].rpc_url, 'http://example.invalid/rpc');
  } finally {
    process.env.MY_RPC = prev;
  }
});

test('loadRunnerConfig fails fast with pinpointed path on invalid chains rpc_url', async () => {
  const prev = process.env.MISSING_RPC;
  delete process.env.MISSING_RPC;
  try {
    const p = tmpFile('bad-rpc.yaml');
    await writeFile(
      p,
      [
        'schema: "ais-runner/0.0.1"',
        'chains:',
        '  "eip155:1":',
        '    rpc_url: "${MISSING_RPC}"',
        '',
      ].join('\n'),
      'utf-8'
    );

    await assert.rejects(
      async () => loadRunnerConfig(p),
      (e) => String(e).includes('chains."eip155:1".rpc_url')
    );
  } finally {
    process.env.MISSING_RPC = prev;
  }
});

test('loadRunnerConfig fails fast with pinpointed path on invalid signer config', async () => {
  const p = tmpFile('bad-signer.yaml');
  await writeFile(
    p,
    [
      'schema: "ais-runner/0.0.1"',
      'chains:',
      '  "eip155:1":',
      '    rpc_url: "http://example.invalid/rpc"',
      '    signer:',
      '      type: "evm_private_key"',
      '',
    ].join('\n'),
    'utf-8'
  );

  await assert.rejects(
    async () => loadRunnerConfig(p),
    (e) =>
      String(e).includes('chains."eip155:1".signer') &&
      String(e).toLowerCase().includes('private_key')
  );
});

