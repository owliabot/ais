import { describe, expect, it } from 'vitest';
import { validateRunnerCommand } from '../src/index.js';

describe('engine commands schema', () => {
  it('accepts apply_patches command with required envelope fields', () => {
    const result = validateRunnerCommand({
      id: 'cmd-1',
      ts: '2026-02-12T00:00:00.000Z',
      kind: 'apply_patches',
      payload: {
        patches: [{ op: 'set', path: 'inputs.amount', value: '1' }],
      },
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.command.kind).toBe('apply_patches');
      expect(result.command.payload.patches).toHaveLength(1);
    }
  });

  it('rejects malformed payload with field_path', () => {
    const result = validateRunnerCommand({
      id: 'cmd-2',
      ts: '2026-02-12T00:00:00.000Z',
      kind: 'user_confirm',
      payload: { approve: true },
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.field_path).toContain('payload');
      expect(result.error.reason.length).toBeGreaterThan(0);
    }
  });
});
