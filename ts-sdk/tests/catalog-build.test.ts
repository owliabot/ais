import { describe, expect, it } from 'vitest';
import { buildCatalog, type DirectoryLoadResult, type ProtocolSpec, type Pack } from '../src/index.js';

function makeProtocol(id: string, version: string, overrides: Partial<ProtocolSpec> = {}): ProtocolSpec {
  return {
    schema: 'ais/0.0.2',
    meta: { protocol: id as any, version, extensions: {} },
    deployments: [{ chain: 'eip155:1' as any, contracts: { router: '0x' + '11'.repeat(20) }, extensions: {} }],
    actions: {
      swap: {
        description: 'swap',
        risk_level: 3,
        risk_tags: ['swap'],
        requires_queries: ['quote'],
        capabilities_required: ['evm_call'],
        execution: {
          'eip155:*': {
            type: 'evm_call',
            to: { lit: '0x' + '11'.repeat(20) } as any,
            abi: { type: 'function', name: 'swap', inputs: [], outputs: [] } as any,
            args: {},
          },
        } as any,
        extensions: {},
      } as any,
    },
    queries: {
      quote: {
        description: 'quote',
        params: [],
        returns: [{ name: 'out', type: 'uint256', extensions: {} } as any],
        execution: {
          'eip155:*': {
            type: 'evm_read',
            to: { lit: '0x' + '11'.repeat(20) } as any,
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] } as any,
            args: {},
          },
        } as any,
        extensions: {},
      } as any,
    },
    extensions: {},
    ...overrides,
  };
}

function makePack(name: string, version: string): Pack {
  return {
    schema: 'ais-pack/0.0.2',
    meta: { name, version, extensions: {} },
    includes: [{ protocol: 'demo', version: '0.0.2', extensions: {} } as any],
    extensions: {},
  } as any;
}

describe('AGT101 catalog builder', () => {
  it('builds stable, hashable catalog with sorted cards', () => {
    const ws: DirectoryLoadResult = {
      protocols: [
        { path: '/ws/p2.yaml', document: makeProtocol('p2', '0.0.1') },
        { path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') },
      ],
      packs: [{ path: '/ws/pack.yaml', document: makePack('pack', '1.0.0') }],
      workflows: [],
      errors: [],
    };

    const c1 = buildCatalog(ws);
    const c2 = buildCatalog(ws);

    expect(c1.schema).toBe('ais-catalog/0.0.1');
    expect(c1.actions.length).toBe(2);
    expect(c1.queries.length).toBe(2);
    expect(c1.packs.length).toBe(1);

    // Stable ordering (protocol asc)
    expect(c1.actions.map((a) => a.protocol)).toEqual(['p1', 'p2']);
    expect(c1.hash).toBe(c2.hash);
  });
});

