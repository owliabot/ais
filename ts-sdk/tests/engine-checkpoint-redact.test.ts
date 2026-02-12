import { describe, expect, it } from 'vitest';
import { serializeCheckpoint, deserializeCheckpoint, type EngineCheckpoint } from '../src/index.js';

describe('AGT201 checkpoint redaction', () => {
  const checkpoint: EngineCheckpoint = {
    schema: 'ais-engine-checkpoint/0.0.2',
    created_at: '2020-01-01T00:00:00.000Z',
    plan: { schema: 'ais-plan/0.0.3', nodes: [], extensions: {} } as any,
    runtime: {
      ctx: {
        private_key: '0xabc',
        api_token: 'secret',
        token_address: '0x' + '11'.repeat(20),
      },
    },
    completed_node_ids: [],
  } as any;

  it('default mode redacts secrets but keeps non-secret token_* fields', () => {
    const raw = serializeCheckpoint(checkpoint, { pretty: false, redact_mode: 'default' });
    expect(raw).toContain('"private_key":"[REDACTED]"');
    expect(raw).toContain('"api_token":"[REDACTED]"');
    // Should NOT redact token_address.
    expect(raw).toContain('"token_address":"0x');
  });

  it('off mode preserves secrets (unsafe)', () => {
    const raw = serializeCheckpoint(checkpoint, { pretty: false, redact_mode: 'off' });
    expect(raw).toContain('"private_key":"0xabc"');
  });

  it('deserializeCheckpoint still works on redacted payload', () => {
    const raw = serializeCheckpoint(checkpoint, { pretty: false, redact_mode: 'default' });
    const restored = deserializeCheckpoint(raw);
    expect(restored.schema).toBe('ais-engine-checkpoint/0.0.2');
  });
});

