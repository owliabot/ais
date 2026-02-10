import { describe, it, expect } from 'vitest';
import {
  serializeCheckpoint,
  deserializeCheckpoint,
  type EngineCheckpoint,
} from '../src/index.js';

describe('checkpoint codec', () => {
  it('roundtrips BigInt and Uint8Array in runtime and plan', () => {
    const checkpoint: EngineCheckpoint = {
      schema: 'ais-engine-checkpoint/0.0.2',
      created_at: new Date(0).toISOString(),
      plan: {
        schema: 'ais-plan/0.0.2',
        nodes: [
          {
            id: 'q1',
            chain: 'eip155:1',
            kind: 'execution',
            execution: {
              type: 'evm_read',
              to: { lit: '0x1111111111111111111111111111111111111111' } as any,
              abi: { type: 'function', name: 'q', inputs: [], outputs: [] } as any,
              args: {},
            } as any,
          },
        ],
      } as any,
      runtime: {
        inputs: { amount: 7n },
        nodes: {
          q1: { outputs: { y: 5n, bytes: new Uint8Array([1, 2, 3]) } },
        },
      },
      completed_node_ids: ['q1'],
      poll_state_by_node_id: {
        q1: { attempts: 2, started_at_ms: 1, next_attempt_at_ms: 2 },
      },
      paused_by_node_id: {
        q2: { reason: 'need_user_confirm', paused_at_ms: 3, details: { want: 1n } },
      },
    };

    const json = serializeCheckpoint(checkpoint);
    const restored = deserializeCheckpoint(json);

    expect(typeof (restored.runtime as any).inputs.amount).toBe('bigint');
    expect((restored.runtime as any).inputs.amount).toBe(7n);
    expect((restored.runtime as any).nodes.q1.outputs.y).toBe(5n);
    expect((restored.runtime as any).nodes.q1.outputs.bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from((restored.runtime as any).nodes.q1.outputs.bytes)).toEqual([1, 2, 3]);
    expect((restored.paused_by_node_id as any).q2.details.want).toBe(1n);
  });

  it('does not revive lookalike tagged objects with extra keys', () => {
    const checkpoint: EngineCheckpoint = {
      schema: 'ais-engine-checkpoint/0.0.2',
      created_at: new Date(0).toISOString(),
      plan: { schema: 'ais-plan/0.0.2', nodes: [] } as any,
      runtime: {
        suspicious: { __ais_json_type: 'bigint', value: '7', extra: true },
      },
      completed_node_ids: [],
    };

    const restored = deserializeCheckpoint(serializeCheckpoint(checkpoint));
    expect((restored.runtime as any).suspicious).toEqual({
      __ais_json_type: 'bigint',
      value: '7',
      extra: true,
    });
  });
});

