import { describe, it, expect } from 'vitest';
import {
  AIS_JSON_CODEC_PROFILE,
  stringifyAisJson,
  parseAisJson,
  serializeCheckpoint,
  deserializeCheckpoint,
  encodeEngineEventJsonlRecord,
  decodeEngineEventJsonlRecord,
  type ExecutionPlan,
  type EngineEvent,
  type RuntimePatch,
  type EngineCheckpoint,
} from '../src/index.js';

describe('AIS JSON codec profile (AGT007A)', () => {
  it('publishes a single canonical profile for bigint/bytes/error', () => {
    expect(AIS_JSON_CODEC_PROFILE.version).toBe('ais-json/1');
    expect(AIS_JSON_CODEC_PROFILE.bigint).toEqual({ tag: 'bigint', format: 'decimal_string' });
    expect(AIS_JSON_CODEC_PROFILE.bytes).toEqual({ tag: 'uint8array', encoding: 'base64' });
    expect(AIS_JSON_CODEC_PROFILE.error.stack_default).toBe('strip');
  });

  it('roundtrips bigint', () => {
    const restored = parseAisJson(stringifyAisJson({ value: 123n })) as any;
    expect(restored.value).toBe(123n);
  });

  it('roundtrips Uint8Array with base64 form', () => {
    const restored = parseAisJson(stringifyAisJson({ bytes: new Uint8Array([1, 2, 3]) })) as any;
    expect(restored.bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(restored.bytes)).toEqual([1, 2, 3]);
  });

  it('encodes Error with stack stripped by default', () => {
    const err = new Error('boom');
    err.stack = 'stack-line';
    const restored = parseAisJson(stringifyAisJson({ error: err })) as any;
    expect(restored.error).toBeInstanceOf(Error);
    expect(restored.error.message).toBe('boom');
    expect(restored.error.stack).not.toBe('stack-line');
  });

  it('can include Error stack when enabled', () => {
    const err = new Error('boom');
    err.stack = 'stack-line';
    const restored = parseAisJson(stringifyAisJson({ error: err }, { include_error_stack: true })) as any;
    expect(restored.error.stack).toBe('stack-line');
  });

  it('does not revive lookalike tagged error objects with extra keys', () => {
    const restored = parseAisJson(
      '{"v":{"__ais_json_type":"error","name":"Error","message":"x","stack":"s","extra":true}}'
    ) as any;
    expect(restored.v).toEqual({
      __ais_json_type: 'error',
      name: 'Error',
      message: 'x',
      stack: 's',
      extra: true,
    });
  });

  it('rejects undefined values when strict option is enabled', () => {
    expect(() => stringifyAisJson({ a: undefined }, { reject_undefined: true })).toThrow(/undefined value/i);
  });

  it('rejects non-finite numbers when strict option is enabled', () => {
    expect(() => stringifyAisJson({ a: Number.NaN }, { reject_non_finite_number: true })).toThrow(
      /non-finite number/i
    );
  });
});

describe('AIS JSON codec roundtrip across plan/event/patch/checkpoint (AGT007)', () => {
  it('roundtrips execution plan payload through codec', () => {
    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.3',
      nodes: [
        {
          id: 'n1',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: { lit: '0x1111111111111111111111111111111111111111' } as any,
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] } as any,
            args: { x: { lit: 7n } },
          },
        },
      ],
    };
    const restored = parseAisJson(stringifyAisJson(plan)) as ExecutionPlan;
    expect((restored.nodes[0] as any).execution.args.x.lit).toBe(7n);
  });

  it('roundtrips engine event JSONL record', () => {
    const ev: EngineEvent = {
      type: 'query_result',
      node: { id: 'n1', chain: 'eip155:1', kind: 'execution', execution: { type: 'evm_read' } as any },
      outputs: { out: 9n, bytes: new Uint8Array([9, 8, 7]) },
    };
    const line = encodeEngineEventJsonlRecord({
      schema: 'ais-engine-event/0.0.3',
      run_id: 'r1',
      seq: 1,
      ts: new Date(0).toISOString(),
      event: { type: ev.type, node_id: ev.node.id, data: { outputs: ev.outputs } },
    });
    const rec = decodeEngineEventJsonlRecord(line);
    expect((rec.event.data as any).outputs.out).toBe(9n);
    expect(Array.from((rec.event.data as any).outputs.bytes)).toEqual([9, 8, 7]);
  });

  it('roundtrips runtime patch payload', () => {
    const patch: RuntimePatch = { op: 'set', path: 'ctx.quote', value: { amount: 3n } };
    const restored = parseAisJson(stringifyAisJson(patch)) as RuntimePatch;
    expect((restored.value as any).amount).toBe(3n);
  });

  it('roundtrips checkpoint payload', () => {
    const checkpoint: EngineCheckpoint = {
      schema: 'ais-engine-checkpoint/0.0.2',
      created_at: new Date(0).toISOString(),
      plan: { schema: 'ais-plan/0.0.3', nodes: [] } as any,
      runtime: { inputs: { amount: 1n } },
      completed_node_ids: ['n1'],
    };
    const restored = deserializeCheckpoint(serializeCheckpoint(checkpoint));
    expect((restored.runtime as any).inputs.amount).toBe(1n);
  });
});
