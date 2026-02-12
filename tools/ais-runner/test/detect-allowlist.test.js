import test from 'node:test';
import assert from 'node:assert/strict';

import { createRunnerDetectResolver } from '../dist/detect.js';

class ValueRefEvalError extends Error {
  constructor(message, options) {
    super(message);
    this.name = 'ValueRefEvalError';
    this.cause = options?.cause;
  }
}

function makeWorkflow() {
  return {
    schema: 'ais-flow/0.0.3',
    meta: { name: 'wf', version: '0.0.3' },
    requires_pack: { name: 'pack-demo', version: '1.0.0' },
    nodes: [],
  };
}

function makeWorkspace(providerConfig) {
  return {
    protocols: [],
    workflows: [],
    errors: [],
    packs: [
      {
        path: 'pack-demo.yaml',
        document: {
          schema: 'ais-pack/0.0.2',
          meta: { name: 'pack-demo', version: '1.0.0' },
          includes: [],
          providers: {
            detect: {
              enabled: [providerConfig],
            },
          },
        },
      },
    ],
  };
}

test('detect allowlist fixture: allow provider selected by SDK', () => {
  const workflow = makeWorkflow();
  const workspaceDocs = makeWorkspace({
    kind: 'best_quote',
    provider: 'pack-provider',
    priority: 20,
    chains: ['eip155:1'],
    candidates: ['pack-first'],
  });
  const calls = [];
  const resolver = createRunnerDetectResolver({
    sdk: {
      ValueRefEvalError,
      pickDetectProvider: (_pack, input) => {
        calls.push(input);
        return { ok: true, kind: 'ok', provider: 'pack-provider' };
      },
    },
    workflow,
    workspaceDocs,
  });

  const value = resolver.resolve(
    { kind: 'best_quote' },
    { runtime: { ctx: { chain_id: 'eip155:1' } } }
  );
  assert.equal(value, 'pack-first');
  assert.equal(calls.length, 1);
  assert.equal(calls[0].kind, 'best_quote');
  assert.equal(calls[0].chain, 'eip155:1');
});

test('detect allowlist fixture: deny provider blocked by SDK', () => {
  const workflow = makeWorkflow();
  const workspaceDocs = makeWorkspace({
    kind: 'best_quote',
    provider: 'pack-provider',
    priority: 20,
    candidates: ['pack-first'],
  });
  const resolver = createRunnerDetectResolver({
    sdk: {
      ValueRefEvalError,
      pickDetectProvider: () => ({
        ok: false,
        kind: 'hard_block',
        reason: 'detect provider is not allowlisted by pack',
        details: { provider: 'forbidden-provider' },
      }),
    },
    workflow,
    workspaceDocs,
  });

  assert.throws(
    () => resolver.resolve({ kind: 'best_quote' }, { runtime: { ctx: { chain_id: 'eip155:1' } } }),
    /not allowlisted/
  );
});

test('detect allowlist fixture: chain dimension deny from SDK', () => {
  const workflow = makeWorkflow();
  const workspaceDocs = makeWorkspace({
    kind: 'best_quote',
    provider: 'pack-provider',
    priority: 20,
    chains: ['eip155:1'],
    candidates: ['pack-first'],
  });
  const resolver = createRunnerDetectResolver({
    sdk: {
      ValueRefEvalError,
      pickDetectProvider: () => ({
        ok: false,
        kind: 'hard_block',
        reason: 'no detect provider available for kind/chain in pack allowlist',
        details: { kind: 'best_quote', chain: 'eip155:8453' },
      }),
    },
    workflow,
    workspaceDocs,
  });

  assert.throws(
    () => resolver.resolve({ kind: 'best_quote' }, { runtime: { ctx: { chain_id: 'eip155:8453' } } }),
    /kind\/chain/
  );
});
