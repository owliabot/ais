import { describe, expect, it } from 'vitest';
import type { Pack, PolicyGateInput } from '../src/index.js';
import { enforcePolicyGate } from '../src/index.js';

function makePack(overrides: Partial<Pack> = {}): Pack {
  return {
    schema: 'ais-pack/0.0.2',
    meta: { name: 'pack-demo', version: '1.0.0' },
    includes: [],
    ...overrides,
  };
}

function makeInput(overrides: Partial<PolicyGateInput> = {}): PolicyGateInput {
  return {
    chain: 'eip155:1',
    action_ref: 'demo@0.0.2/swap',
    action_key: 'demo.swap',
    risk_level: 3,
    risk_tags: ['swap'],
    spend_amount: '1000',
    slippage_bps: 50,
    token_address: '0x1111111111111111111111111111111111111111',
    ...overrides,
  };
}

describe('enforcePolicyGate risk scenarios (AGT005)', () => {
  it('returns ok when pack policy is missing', () => {
    const result = enforcePolicyGate(undefined, makeInput());
    expect(result.ok).toBe(true);
    expect(result.kind).toBe('ok');
  });

  it('hard-blocks when slippage exceeds max', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: { hard_constraints_defaults: { max_slippage_bps: 30 } },
      }),
      makeInput({ slippage_bps: 80 })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).violations[0].field).toBe('slippage_bps');
  });

  it('hard-blocks unlimited approval when disallowed', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: { hard_constraints_defaults: { allow_unlimited_approval: false } },
      }),
      makeInput({
        action_ref: 'demo@0.0.2/approve',
        action_key: 'demo.approve',
        approval_amount: '1000000000000000000',
        unlimited_approval: true,
      })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).violations[0].field).toBe('unlimited_approval');
  });

  it('requires confirm when risk level crosses approval threshold', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: { approvals: { auto_execute_max_risk_level: 2, require_approval_min_risk_level: 3 } },
      }),
      makeInput({ risk_level: 4 })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect((result.details as any).approval_reasons.length).toBeGreaterThan(0);
  });

  it('hard-blocks token not in allowlist under strict mode', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: {},
        token_policy: {
          resolution: { require_allowlist_for_symbol_resolution: true },
          allowlist: [{ chain: 'eip155:1', symbol: 'USDC', address: '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }],
        },
      }),
      makeInput({ token_address: '0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).violations[0].constraint).toContain('token_policy.allowlist');
  });

  it('requires confirm token not in allowlist under permissive mode', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: {},
        token_policy: {
          resolution: { require_allowlist_for_symbol_resolution: false },
          allowlist: [{ chain: 'eip155:1', symbol: 'USDC', address: '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }],
        },
      }),
      makeInput({ token_address: '0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect((result.details as any).approval_reasons[0]).toContain('allowlist');
  });

  it('requires confirm when missing_fields exist', () => {
    const result = enforcePolicyGate(
      makePack({ policy: {} }),
      makeInput({ missing_fields: ['slippage_bps'], slippage_bps: undefined })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect((result.details as any).missing_fields).toEqual(['slippage_bps']);
  });

  it('requires confirm when unknown_fields exist', () => {
    const result = enforcePolicyGate(
      makePack({ policy: {} }),
      makeInput({ unknown_fields: ['token_identity'] })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect((result.details as any).unknown_fields).toEqual(['token_identity']);
  });

  it('hard-blocks with multiple violations when multiple hard constraints are hit', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: {
          hard_constraints_defaults: {
            max_slippage_bps: 10,
            allow_unlimited_approval: false,
          },
        },
      }),
      makeInput({
        slippage_bps: 99,
        approval_amount: '100',
        unlimited_approval: true,
      })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).violations.length).toBeGreaterThanOrEqual(2);
  });

  it('requires confirm when legacy approval_required tags match', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: { approval_required: ['bridge', 'high_impact'] },
      }),
      makeInput({ risk_tags: ['swap', 'bridge'] })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect((result.details as any).approval_reasons[0]).toContain('Risk tags require approval');
  });

  it('keeps hard_block precedence over approval-required reasons', () => {
    const result = enforcePolicyGate(
      makePack({
        policy: {
          approvals: { require_approval_min_risk_level: 3 },
          hard_constraints_defaults: { max_slippage_bps: 30 },
        },
      }),
      makeInput({ risk_level: 5, slippage_bps: 100 })
    );
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect((result.details as any).violations[0].field).toBe('slippage_bps');
  });
});
