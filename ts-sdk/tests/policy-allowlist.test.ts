import { describe, expect, it } from 'vitest';
import type { Pack } from '../src/index.js';
import { checkDetectAllowed, checkExecutionPluginAllowed, pickDetectProvider } from '../src/index.js';

function makePack(): Pack {
  return {
    schema: 'ais-pack/0.0.2',
    meta: { name: 'pack-demo', version: '1.0.0' },
    includes: [],
    providers: {
      detect: {
        enabled: [
          { kind: 'best_quote', provider: 'p1', priority: 10, chains: ['eip155:1'] },
          { kind: 'best_quote', provider: 'p2', priority: 20, chains: ['eip155:1'] },
          { kind: 'best_quote', provider: 'p3', priority: 30, chains: ['eip155:8453'] },
        ],
      },
    },
    plugins: {
      execution: {
        enabled: [{ type: 'custom_plugin', chains: ['eip155:1'] }],
      },
    },
  };
}

describe('policy allowlist (AGT004A/AGT004B)', () => {
  it('pickDetectProvider chooses highest-priority provider in allowlist for chain', () => {
    const picked = pickDetectProvider(makePack(), {
      kind: 'best_quote',
      chain: 'eip155:1',
    });
    expect(picked.ok).toBe(true);
    expect(picked.provider).toBe('p2');
  });

  it('pickDetectProvider filters by candidates and returns structured details on miss', () => {
    const picked = pickDetectProvider(makePack(), {
      kind: 'best_quote',
      chain: 'eip155:1',
      candidates: ['unknown-provider'],
    });
    expect(picked.ok).toBe(false);
    expect(picked.reason).toBe('no detect provider available for kind/chain in pack allowlist');
    expect(picked.details).toMatchObject({
      kind: 'best_quote',
      chain: 'eip155:1',
      candidates: ['unknown-provider'],
    });
    expect(Array.isArray((picked.details as any).pack_enabled_for_kind)).toBe(true);
    expect(Array.isArray((picked.details as any).eligible_for_chain)).toBe(true);
  });

  it('checkDetectAllowed returns structured provider mismatch details', () => {
    const result = checkDetectAllowed(makePack(), {
      kind: 'best_quote',
      provider: 'forbidden',
      chain: 'eip155:1',
    });
    expect(result.ok).toBe(false);
    expect(result.reason).toBe('detect provider is not allowlisted by pack');
    expect(result.details).toMatchObject({
      kind: 'best_quote',
      provider: 'forbidden',
      chain: 'eip155:1',
      pack_meta: { name: 'pack-demo', version: '1.0.0' },
    });
    expect(Array.isArray((result.details as any).pack_enabled_for_kind)).toBe(true);
  });

  it('checkExecutionPluginAllowed enforces type+chain and reports allowed list', () => {
    const denied = checkExecutionPluginAllowed(makePack(), {
      type: 'custom_plugin',
      chain: 'eip155:8453',
    });
    expect(denied.ok).toBe(false);
    expect(denied.reason).toBe('plugin execution type is not allowlisted by pack');
    expect(denied.details).toMatchObject({
      type: 'custom_plugin',
      chain: 'eip155:8453',
      pack_meta: { name: 'pack-demo', version: '1.0.0' },
    });
    expect(Array.isArray((denied.details as any).allowed)).toBe(true);
  });
});
