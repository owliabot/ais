import { describe, it, expect } from 'vitest';
import {
  encodeEngineEventJsonlRecord,
  decodeEngineEventJsonlRecord,
  engineEventToEnvelope,
  createJsonlRpcPeer,
  stringifyAisJson,
} from '../src/index.js';
import { Readable, PassThrough } from 'node:stream';

describe('T443 JSONL/RPC adapters', () => {
  it('encodes/decodes EngineEvent JSONL records with bigint/uint8array/error', () => {
    const rec = {
      schema: 'ais-engine-event/0.0.3' as const,
      run_id: 'run-1',
      seq: 0,
      ts: new Date().toISOString(),
      event: {
        type: 'error' as const,
        data: {
          reason: 'boom',
          retryable: true,
          error: new Error('boom'),
          outputs: { x: 7n, bytes: new Uint8Array([1, 2, 3]) },
        },
      },
    };

    const line = encodeEngineEventJsonlRecord(rec);
    const parsed = decodeEngineEventJsonlRecord(line);
    const ev = parsed.event as any;

    expect(ev.data.outputs.x).toBe(7n);
    expect(ev.data.outputs.bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(ev.data.outputs.bytes)).toEqual([1, 2, 3]);
    expect(ev.data.error).toBeInstanceOf(Error);
    expect(ev.data.error.message).toBe('boom');
    expect(ev.data.retryable).toBe(true);
    expect(ev.data.reason).toBe('boom');
  });

  it('converts EngineEvent to normalized envelope shape', () => {
    const envelope = engineEventToEnvelope({
      type: 'need_user_confirm',
      node: {
        id: 'n1',
        chain: 'eip155:1',
        kind: 'execution',
        execution: { type: 'evm_call' } as any,
      } as any,
      reason: 'policy approval required',
      details: { hit_reasons: ['risk'] },
    });

    expect(envelope.type).toBe('need_user_confirm');
    expect(envelope.node_id).toBe('n1');
    expect(envelope.data).toEqual({
      reason: 'policy approval required',
      details: { hit_reasons: ['risk'] },
    });
  });

  it('JSONL RPC peer transports bigint safely', async () => {
    const input = Readable.from([`${stringifyAisJson({ a: 1n })}\n`]);
    const output = new PassThrough();
    const chunks: Buffer[] = [];
    output.on('data', (c) => chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(String(c))));
    const peer = createJsonlRpcPeer({ input, output });

    const msgs: unknown[] = [];
    for await (const m of peer.messages()) msgs.push(m);
    expect((msgs[0] as any).a).toBe(1n);

    peer.send({ b: 2n });
    await new Promise<void>((r) => output.end(r));
    const written = Buffer.concat(chunks).toString('utf-8');
    expect(written.includes('__ais_json_type')).toBe(true);
  });
});
