import { describe, it, expect } from 'vitest';
import { PassThrough } from 'node:stream';
import { createJsonlTraceSinkFromWritable, parseAisJson } from '../src/index.js';

describe('trace JSONL sink uses AIS codec (AGT007B)', () => {
  it('writes trace line with bigint preserved by tagged representation', async () => {
    const output = new PassThrough();
    const chunks: Buffer[] = [];
    output.on('data', (c) => chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(String(c))));

    const sink = createJsonlTraceSinkFromWritable(output);
    sink.append({
      kind: 'event',
      id: 'e1',
      run_id: 'r1',
      seq: 1,
      ts: new Date(0).toISOString(),
      data: { amount: 42n },
    });
    await new Promise<void>((resolve) => output.end(resolve));

    const line = Buffer.concat(chunks).toString('utf-8').trim();
    const restored = parseAisJson(line) as any;
    expect(restored.data.amount).toBe(42n);
  });
});
