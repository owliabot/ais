import { describe, it, expect } from 'vitest';
import {
  encodeEngineEventJsonlRecord,
  decodeEngineEventJsonlRecord,
  createJsonlRpcPeer,
  stringifyAisJson,
} from '../src/index.js';
import { Readable, PassThrough } from 'node:stream';

describe('T443 JSONL/RPC adapters', () => {
  it('encodes/decodes EngineEvent JSONL records with bigint/uint8array/error', () => {
    const rec = {
      schema: 'ais-engine-event/0.0.2' as const,
      run_id: 'run-1',
      seq: 0,
      ts: new Date().toISOString(),
      event: {
        type: 'error' as const,
        error: new Error('boom'),
        outputs: { x: 7n, bytes: new Uint8Array([1, 2, 3]) },
      },
    };

    const line = encodeEngineEventJsonlRecord(rec);
    const parsed = decodeEngineEventJsonlRecord(line);
    const ev = parsed.event as any;

    expect(ev.outputs.x).toBe(7n);
    expect(ev.outputs.bytes).toBeInstanceOf(Uint8Array);
    expect(Array.from(ev.outputs.bytes)).toEqual([1, 2, 3]);
    expect(ev.error).toBeInstanceOf(Error);
    expect(ev.error.message).toBe('boom');
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
