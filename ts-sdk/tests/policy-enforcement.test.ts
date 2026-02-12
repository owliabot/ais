import { describe, expect, it } from 'vitest';
import type { ExecutionPlanNode, ExecutionSpec, Pack } from '../src/index.js';
import {
  compileWritePreview,
  createContext,
  enforcePolicyGate,
  extractPolicyGateInput,
} from '../src/index.js';

function makeNode(execution: ExecutionSpec, chain: string, action = 'testAction'): ExecutionPlanNode {
  return {
    id: `node_${action}`,
    chain,
    kind: 'action_ref',
    execution,
    source: {
      protocol: 'demo/v1.0.0',
      action,
      node_id: `wf_${action}`,
    },
  };
}

describe('policy enforcement extraction (AGT010B/AGT010C)', () => {
  it('builds EVM approve preview and extracts approval fields', () => {
    const ctx = createContext();
    const maxUintHex = '0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff';
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: {
          type: 'function',
          name: 'approve',
          inputs: [
            { name: 'spender', type: 'address' },
            { name: 'amount', type: 'uint256' },
          ],
          outputs: [],
        },
        args: {
          spender: { lit: '0x2222222222222222222222222222222222222222' },
          amount: { lit: maxUintHex },
        },
      },
      'eip155:1',
      'approve'
    );

    const preview = compileWritePreview({ node, ctx, resolved_params: {} });
    expect(preview.kind).toBe('evm_tx');
    expect(preview.function_name).toBe('approve');

    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {}, preview });
    expect(gate.token_address).toBe('0x1111111111111111111111111111111111111111');
    expect(gate.spender_address).toBe('0x2222222222222222222222222222222222222222');
    expect(gate.approval_amount).toBe(
      '115792089237316195423570985008687907853269984665640564039457584007913129639935'
    );
    expect(gate.unlimited_approval).toBe(true);
  });

  it('extracts EVM swap spend/slippage from preview args', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x3333333333333333333333333333333333333333' },
        abi: {
          type: 'function',
          name: 'swapExactTokensForTokens',
          inputs: [
            { name: 'amountIn', type: 'uint256' },
            { name: 'amountOutMin', type: 'uint256' },
            { name: 'path', type: 'address[]' },
            { name: 'to', type: 'address' },
            { name: 'deadline', type: 'uint256' },
            { name: 'slippageBps', type: 'uint256' },
          ],
          outputs: [],
        },
        args: {
          amountIn: { lit: '12345' },
          amountOutMin: { lit: '12000' },
          path: {
            array: [
              { lit: '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
              { lit: '0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' },
            ],
          },
          to: { lit: '0x4444444444444444444444444444444444444444' },
          deadline: { lit: '9999999999' },
          slippageBps: { lit: '80' },
        },
      },
      'eip155:1',
      'swap'
    );

    const preview = compileWritePreview({ node, ctx, resolved_params: {} });
    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {}, preview });
    expect(gate.spend_amount).toBe('12345');
    expect(gate.slippage_bps).toBe(80);
  });

  it('requires confirm when approve fields are missing', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x5555555555555555555555555555555555555555' },
        abi: {
          type: 'function',
          name: 'approve',
          inputs: [{ name: 'foo', type: 'uint256' }],
          outputs: [],
        },
        args: {
          foo: { lit: '1' },
        },
      },
      'eip155:1',
      'approve_missing'
    );

    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {} });
    const result = enforcePolicyGate({ policy: {} } as Pack, gate);
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
    expect(result.reason).toContain('incomplete');
  });

  it('builds Solana preview with program/accounts/data summary', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'solana_instruction',
        program: { lit: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
        instruction: 'approve',
        accounts: [
          { name: 'source', pubkey: { lit: '2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'delegate', pubkey: { lit: '7T4C6fWGh7kr8o5A2mQq1L6kz3b4z9xT2rG8Qk3pE7cR' }, signer: { lit: false }, writable: { lit: false } },
          { name: 'owner', pubkey: { lit: '6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV' }, signer: { lit: true }, writable: { lit: false } },
          { name: 'mint', pubkey: { lit: '9Q2sV4rM1kT7hL3fN8pW5xY6cD2bJ4gR1uE7aC9mP5tK' }, signer: { lit: false }, writable: { lit: false } },
        ],
        data: {
          object: {
            amount: { lit: '1000' },
          },
        },
      },
      'solana:mainnet',
      'solana_approve'
    );

    const preview = compileWritePreview({ node, ctx, resolved_params: {} });
    expect(preview.kind).toBe('solana_instruction');
    expect(preview.program_id).toBe('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
    expect(Array.isArray(preview.accounts)).toBe(true);
    expect(preview.data_summary).toMatchObject({ type: 'object' });
  });

  it('extracts Solana transfer spend/token fields from preview', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'solana_instruction',
        program: { lit: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
        instruction: 'transfer',
        accounts: [
          { name: 'source', pubkey: { lit: '2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'destination', pubkey: { lit: '3N5iQ6vW7xY8zA9bC1dE2fG3hJ4kL5mN6pQ7rS8tU9vW' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'owner', pubkey: { lit: '6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV' }, signer: { lit: true }, writable: { lit: false } },
        ],
        data: {
          object: {
            amount: { lit: '42' },
          },
        },
      },
      'solana:mainnet',
      'solana_transfer'
    );

    const preview = compileWritePreview({ node, ctx, resolved_params: {} });
    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {}, preview });
    expect(gate.spend_amount).toBe('42');
    expect(gate.token_address).toBe('2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP');
    expect(gate.owner_address).toBe('6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV');
  });

  it('keeps params priority over preview fields', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x8888888888888888888888888888888888888888' },
        abi: {
          type: 'function',
          name: 'approve',
          inputs: [
            { name: 'spender', type: 'address' },
            { name: 'amount', type: 'uint256' },
          ],
          outputs: [],
        },
        args: {
          spender: { lit: '0x9999999999999999999999999999999999999999' },
          amount: { lit: '123' },
        },
      },
      'eip155:1',
      'approve_priority'
    );
    const gate = extractPolicyGateInput({
      node,
      ctx,
      resolved_params: {
        approval_amount: '777',
      },
    });
    expect(gate.approval_amount).toBe('777');
    expect(gate.field_sources?.approval_amount).toContain('params');
  });

  it('records field_sources from preview extraction', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' },
        abi: {
          type: 'function',
          name: 'swapExactTokensForTokens',
          inputs: [
            { name: 'amountIn', type: 'uint256' },
            { name: 'slippageBps', type: 'uint256' },
          ],
          outputs: [],
        },
        args: {
          amountIn: { lit: '100' },
          slippageBps: { lit: '50' },
        },
      },
      'eip155:1',
      'swap_source'
    );
    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {} });
    expect(gate.field_sources?.spend_amount).toContain('preview.evm');
    expect(gate.field_sources?.slippage_bps).toContain('preview.evm');
  });

  it('marks missing slippage for swap as confirm-required', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x1212121212121212121212121212121212121212' },
        abi: {
          type: 'function',
          name: 'swapExactTokensForTokens',
          inputs: [{ name: 'amountIn', type: 'uint256' }],
          outputs: [],
        },
        args: {
          amountIn: { lit: '100' },
        },
      },
      'eip155:1',
      'swap_missing_slippage'
    );
    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {} });
    expect(gate.missing_fields).toContain('slippage_bps');
    const result = enforcePolicyGate({ policy: {} } as Pack, gate);
    expect(result.ok).toBe(false);
    expect(result.kind).toBe('need_user_confirm');
  });

  it('marks missing delegate for solana approve as confirm-required', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'solana_instruction',
        program: { lit: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
        instruction: 'approve',
        accounts: [
          { name: 'source', pubkey: { lit: '2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'owner', pubkey: { lit: '6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV' }, signer: { lit: true }, writable: { lit: false } },
        ],
        data: {
          object: {
            amount: { lit: '10' },
          },
        },
      },
      'solana:mainnet',
      'solana_approve_missing_delegate'
    );
    const gate = extractPolicyGateInput({
      node,
      ctx,
      resolved_params: {},
      preview: {
        kind: 'solana_instruction',
        chain: 'solana:mainnet',
        instruction: 'approve',
        accounts: [
          { name: 'source', pubkey: '2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP' },
          { name: 'owner', pubkey: '6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV' },
        ],
        data_fields: { amount: '10' },
      },
    });
    expect(gate.missing_fields).toContain('spender_address');
  });

  it('supports transfer_checked extraction on solana', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'solana_instruction',
        program: { lit: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' },
        instruction: 'transfer_checked',
        accounts: [
          { name: 'source', pubkey: { lit: '2M4hP4xg5w4mKTfQqfH9Q6tQ1ZX4Q7u4wJ8YimL2xYhP' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'mint', pubkey: { lit: '9Q2sV4rM1kT7hL3fN8pW5xY6cD2bJ4gR1uE7aC9mP5tK' }, signer: { lit: false }, writable: { lit: false } },
          { name: 'destination', pubkey: { lit: '3N5iQ6vW7xY8zA9bC1dE2fG3hJ4kL5mN6pQ7rS8tU9vW' }, signer: { lit: false }, writable: { lit: true } },
          { name: 'owner', pubkey: { lit: '6v8Jk5eP2xM9aH1tQ3fN4rP7uY2wK8sD4gZ6mL1nC2bV' }, signer: { lit: true }, writable: { lit: false } },
        ],
        data: {
          object: {
            amount: { lit: '88' },
            decimals: { lit: 6 },
          },
        },
      },
      'solana:mainnet',
      'solana_transfer_checked'
    );
    const gate = extractPolicyGateInput({ node, ctx, resolved_params: {} });
    expect(gate.spend_amount).toBe('88');
    expect(gate.token_address).toBe('9Q2sV4rM1kT7hL3fN8pW5xY6cD2bJ4gR1uE7aC9mP5tK');
  });

  it('merges action risk tags with pack override tags', () => {
    const ctx = createContext();
    const node = makeNode(
      {
        type: 'evm_call',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: { type: 'function', name: 'approve', inputs: [{ name: 'spender', type: 'address' }, { name: 'amount', type: 'uint256' }], outputs: [] },
        args: {
          spender: { lit: '0x2222222222222222222222222222222222222222' },
          amount: { lit: '1' },
        },
      },
      'eip155:1',
      'approve'
    );
    const pack = {
      overrides: {
        actions: {
          'demo/v1.0.0.approve': {
            risk_tags: ['high_impact'],
          },
        },
      },
    } as Pack;
    const gate = extractPolicyGateInput({
      node,
      ctx,
      pack,
      resolved_params: {},
      action_risk_tags: ['approval'],
    });
    expect(gate.risk_tags).toEqual(expect.arrayContaining(['approval', 'high_impact']));
  });
});
