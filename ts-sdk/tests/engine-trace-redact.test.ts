import { describe, expect, it } from 'vitest';
import { redactEngineEventByMode, type EngineEvent } from '../src/index.js';

describe('engine redaction mode', () => {
  const baseEvent: EngineEvent = {
    type: 'tx_prepared',
    node: {
      id: 'n1',
      chain: 'eip155:1',
      kind: 'execution',
      execution: { type: 'evm_call' } as any,
    } as any,
    tx: {
      to: '0x1111111111111111111111111111111111111111',
      rpc_payload: { method: 'eth_sendRawTransaction' },
      private_key: '0xabc',
      user_pii: { email: 'user@example.com' },
    },
  };

  it('default mode redacts strict sensitive fields', () => {
    const redacted = redactEngineEventByMode(baseEvent, 'default') as Record<string, unknown>;
    expect(redacted.type).toBe('tx_prepared');
    expect(redacted.tx).toBe('[REDACTED]');
  });

  it('audit mode preserves broader structure but still redacts secrets', () => {
    const redacted = redactEngineEventByMode(baseEvent, 'audit') as Record<string, unknown>;
    const tx = redacted.tx as Record<string, unknown>;
    expect(tx.private_key).toBe('[REDACTED]');
    expect(tx.user_pii).toBe('[REDACTED]');
    expect(tx.rpc_payload).toEqual({ method: 'eth_sendRawTransaction' });
  });

  it('off mode returns original event content', () => {
    const redacted = redactEngineEventByMode(baseEvent, 'off') as Record<string, unknown>;
    const tx = redacted.tx as Record<string, unknown>;
    expect(tx.private_key).toBe('0xabc');
    expect(tx.rpc_payload).toEqual({ method: 'eth_sendRawTransaction' });
  });
});
