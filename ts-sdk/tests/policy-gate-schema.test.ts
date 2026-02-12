import { describe, expect, it } from 'vitest';
import {
  POLICY_GATE_INPUT_FIELD_DICTIONARY,
  POLICY_GATE_OUTPUT_FIELD_DICTIONARY,
  createContext,
  enforcePolicyGate,
  extractPolicyGateInput,
  validatePolicyGateInput,
  validatePolicyGateOutput,
  type ExecutionPlanNode,
  type Pack,
} from '../src/index.js';

describe('policy gate schema (AGT005A)', () => {
  it('validates PolicyGateInput shape', () => {
    const parsed = validatePolicyGateInput({
      chain: 'eip155:1',
      action_key: 'dex.swap',
      spend_amount: '1000000',
      slippage_bps: 50,
      risk_level: 3,
      risk_tags: ['swap'],
    });
    expect(parsed.ok).toBe(true);
  });

  it('rejects invalid PolicyGateInput fields', () => {
    const parsed = validatePolicyGateInput({
      chain: 'eip155:1',
      spend_amount: 123,
    });
    expect(parsed.ok).toBe(false);
  });

  it('validates PolicyGateOutput shape', () => {
    const parsed = validatePolicyGateOutput({
      ok: false,
      kind: 'need_user_confirm',
      reason: 'policy gate input is incomplete',
      details: { missing_fields: ['slippage_bps'] },
    });
    expect(parsed.ok).toBe(true);
  });

  it('contains canonical field dictionary entries', () => {
    expect(POLICY_GATE_INPUT_FIELD_DICTIONARY.chain.null_semantics).toBe('required_missing');
    expect(POLICY_GATE_INPUT_FIELD_DICTIONARY.slippage_bps.required_when).toContain('swap');
    expect(POLICY_GATE_OUTPUT_FIELD_DICTIONARY.kind.value_format).toContain('ok');
  });

  it('hard-blocks when preview compilation is unknown and marked required', () => {
    const ctx = createContext();
    const node: ExecutionPlanNode = {
      id: 'n-unknown',
      kind: 'action_ref',
      chain: 'eip155:1',
      execution: {
        type: 'evm_call',
        to: { lit: 'not-an-address' },
        abi: { type: 'function', name: 'swapExactTokensForTokens', inputs: [], outputs: [] },
        args: {},
      },
      source: {
        protocol: 'demo/v1.0.0',
        action: 'swap',
      },
    };

    const gate = extractPolicyGateInput({
      node,
      ctx,
      resolved_params: {},
    });
    expect(gate.unknown_fields).toEqual(expect.arrayContaining(['preview_compile', 'token_identity']));

    const result = enforcePolicyGate({ policy: {} } as Pack, gate);
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('hard_block');
    expect(result.reason).toContain('required fields');
  });
});
