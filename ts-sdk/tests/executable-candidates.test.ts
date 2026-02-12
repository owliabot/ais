import { describe, expect, it } from 'vitest';
import {
  buildCatalog,
  getExecutableCandidates,
  type DirectoryLoadResult,
  type Pack,
  type ProtocolSpec,
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
        params: [{ name: 'amount', type: 'uint256', required: true, extensions: {} } as any],
        execution: {
          'eip155:1': { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'x', inputs: [], outputs: [] }, args: {} },
          'eip155:137': { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'x', inputs: [], outputs: [] }, args: {} },
        } as any,
        extensions: {},
      } as any,
      a_solana: {
        description: 'solana action',
        risk_level: 2,
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
        enabled: [
          { kind: 'token' as any, provider: 'mock', chains: ['eip155:1' as any, 'eip155:137' as any], priority: 10, extensions: {} } as any,
        ],
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

describe('AGT105A getExecutableCandidates', () => {
  it('returns stable, filtered candidates (pack + engine + chain_scope)', () => {
    const ws: DirectoryLoadResult = {
      protocols: [{ path: '/ws/p1.yaml', document: makeProtocol('p1', '0.0.1') }],
      packs: [],
      workflows: [],
      errors: [],
    };

    const catalog = buildCatalog(ws);
    const pack = makePack('pack', '1.0.0');

    const c = getExecutableCandidates({
      catalog,
      pack,
      engine_capabilities: { execution_types: ['evm_call'], detect_kinds: ['token'] },
      chain_scope: ['eip155:1'],
    });

    expect(c.schema).toBe('ais-executable-candidates/0.0.1');
    expect(c.actions.some((a) => a.id === 'a_solana')).toBe(false);
    expect(c.actions.every((a) => a.execution_chains.every((ch) => ch === 'eip155:1'))).toBe(true);
    expect(c.detect_providers.every((p) => p.chain === 'eip155:1')).toBe(true);
    expect(c.execution_plugins.every((p) => p.type === 'evm_call')).toBe(true);
    expect(c.actions[0]!.signature).toContain('a_evm(');

    // Stable hash for same inputs
    const c2 = getExecutableCandidates({
      catalog,
      pack,
      engine_capabilities: { execution_types: ['evm_call'], detect_kinds: ['token'] },
      chain_scope: ['eip155:1'],
    });
    expect(c.hash).toBe(c2.hash);
  });
});

