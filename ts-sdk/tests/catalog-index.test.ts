import { describe, expect, it } from 'vitest';
import {
  buildCatalog,
  buildCatalogIndex,
  filterByPack,
  filterByEngineCapabilities,
  type DirectoryLoadResult,
  type ProtocolSpec,
  type Pack,
} from '../src/index.js';

function makeProtocol(id: string, version: string, overrides: Partial<ProtocolSpec> = {}): ProtocolSpec {
  return {
    schema: 'ais/0.0.2',
    meta: { protocol: id as any, version, extensions: {} },
    deployments: [{ chain: 'eip155:1' as any, contracts: {}, extensions: {} }],
    actions: {
      a_evm: {
        description: 'evm action',
        risk_level: 2,
        risk_tags: [],
        requires_queries: [],
        execution: {
          'eip155:1': { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'x', inputs: [], outputs: [] }, args: {} },
          'eip155:137': { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'x', inputs: [], outputs: [] }, args: {} },
        } as any,
        extensions: {},
      } as any,
      a_solana: {
        description: 'solana action',
        risk_level: 2,
        risk_tags: [],
        requires_queries: [],
        execution: {
          'solana:mainnet': { type: 'solana_instruction', program_id: { lit: '11111111111111111111111111111111' }, instruction: 'noop', accounts: [], data: { lit: '' } },
        } as any,
        extensions: {},
      } as any,
    },
    queries: {
      q1: {
        description: 'q1',
        params: [],
        returns: [{ name: 'out', type: 'uint256', extensions: {} } as any],
        execution: {
          'eip155:1': { type: 'evm_read', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'q', inputs: [], outputs: [{ name: 'out', type: 'uint256' }] }, args: {} },
        } as any,
        extensions: {},
      } as any,
    },
    extensions: {},
    ...overrides,
  };
}

function makePack(name: string, version: string, overrides: Partial<Pack> = {}): Pack {
  return {
    schema: 'ais-pack/0.0.2',
    meta: { name, version, extensions: {} },
    includes: [{ protocol: 'p1', version: '0.0.1', chain_scope: ['eip155:1' as any], extensions: {} } as any],
    providers: {
      detect: {
        enabled: [{ kind: 'token' as any, provider: 'mock', chains: ['eip155:1' as any], priority: 10, extensions: {} } as any],
        extensions: {},
      },
      extensions: {},
    } as any,
    plugins: {
      execution: {
        enabled: [
          { type: 'evm_call', chains: ['eip155:1' as any], extensions: {} },
          { type: 'solana_instruction', chains: ['solana:mainnet' as any], extensions: {} },
        ],
        extensions: {},
      },
      extensions: {},
    } as any,
    extensions: {},
    ...overrides,
  } as any;
}

describe('AGT105 catalog index + filtering', () => {
  it('buildCatalogIndex provides fast lookups', () => {
    const ws: DirectoryLoadResult = {
      protocols: [{ path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') }],
      packs: [],
      workflows: [],
      errors: [],
    };
    const catalog = buildCatalog(ws);
    const idx = buildCatalogIndex(catalog);
    expect(idx.actions_by_ref.get('p1@0.0.1/a_evm')?.id).toBe('a_evm');
    expect(idx.queries_by_ref.get('p1@0.0.1/q1')?.id).toBe('q1');
    expect(idx.actions_by_protocol_version.get('p1@0.0.1')?.length).toBeGreaterThan(0);
  });

  it('filterByPack enforces includes + chain_scope and derives providers/plugins candidates', () => {
    const ws: DirectoryLoadResult = {
      protocols: [
        { path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') },
        { path: '/ws/p2.yaml', document: makeProtocol('p2', '0.0.1') },
      ],
      packs: [],
      workflows: [],
      errors: [],
    };
    const idx = buildCatalogIndex(buildCatalog(ws));
    const pack = makePack('pack', '1.0.0');
    const filtered = filterByPack(idx, pack);

    // Only p1@0.0.1 remains
    expect(filtered.actions.every((a) => a.protocol === 'p1')).toBe(true);
    expect(filtered.queries.every((q) => q.protocol === 'p1')).toBe(true);

    // chain_scope trims execution_chains
    const a = filtered.actions.find((x) => x.id === 'a_evm');
    expect(a?.execution_chains).toEqual(['eip155:1']);

    // providers/plugins candidates exist (pack boundary)
    expect(filtered.detect_providers?.[0]).toMatchObject({ kind: 'token', provider: 'mock' });
    expect(filtered.execution_plugins?.map((p) => p.type).sort()).toEqual(['evm_call', 'solana_instruction']);
  });

  it('filterByEngineCapabilities filters execution_types and derived providers/plugins', () => {
    const ws: DirectoryLoadResult = {
      protocols: [{ path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') }],
      packs: [],
      workflows: [],
      errors: [],
    };
    const idx = buildCatalogIndex(buildCatalog(ws));
    const packFiltered = filterByPack(idx, makePack('pack', '1.0.0'));

    const filtered = filterByEngineCapabilities(packFiltered, {
      execution_types: ['evm_call'],
      detect_kinds: ['token'],
    });

    expect(filtered.actions.some((a) => a.id === 'a_solana')).toBe(false);
    expect(filtered.execution_plugins?.map((p) => p.type)).toEqual(['evm_call']);
    expect(filtered.detect_providers?.map((p) => p.kind)).toEqual(['token']);
  });

  it('different pack.chain_scope produces different candidate chains (stable + traceable)', () => {
    const ws: DirectoryLoadResult = {
      protocols: [{ path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') }],
      packs: [],
      workflows: [],
      errors: [],
    };
    const idx = buildCatalogIndex(buildCatalog(ws));

    const p1 = makePack('pack', '1.0.0', {
      includes: [{ protocol: 'p1', version: '0.0.1', chain_scope: ['eip155:1' as any], extensions: {} } as any],
    });
    const p2 = makePack('pack', '2.0.0', {
      includes: [{ protocol: 'p1', version: '0.0.1', chain_scope: ['eip155:137' as any], extensions: {} } as any],
    });

    const c1 = filterByPack(idx, p1).actions.find((a) => a.id === 'a_evm')!.execution_chains;
    const c2 = filterByPack(idx, p2).actions.find((a) => a.id === 'a_evm')!.execution_chains;
    expect(c1).toEqual(['eip155:1']);
    expect(c2).toEqual(['eip155:137']);
  });

  it('pack.includes acts as a hard boundary (non-included protocols removed)', () => {
    const ws: DirectoryLoadResult = {
      protocols: [
        { path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') },
        { path: '/ws/p2.yaml', document: makeProtocol('p2', '0.0.1') },
      ],
      packs: [],
      workflows: [],
      errors: [],
    };
    const idx = buildCatalogIndex(buildCatalog(ws));
    const pack = makePack('pack', '1.0.0', {
      includes: [{ protocol: 'p2', version: '0.0.1', extensions: {} } as any],
    });
    const filtered = filterByPack(idx, pack);
    expect(filtered.actions.every((a) => a.protocol === 'p2')).toBe(true);
    expect(filtered.queries.every((q) => q.protocol === 'p2')).toBe(true);
  });
});
