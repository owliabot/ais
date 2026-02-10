import { describe, it, expect } from 'vitest';
import {
  createContext,
  setRef,
  compileEvmCall,
  compileEvmRead,
  type EvmCall,
  type EvmRead,
  EvmCompileError,
} from '../src/index.js';

describe('EVM compiler (T166)', () => {
  it('compiles evm_call with JSON ABI tuple args', () => {
    const ctx = createContext();
    setRef(ctx, 'contracts.router', '0x1111111111111111111111111111111111111111');
    setRef(ctx, 'inputs.tokenIn', '0x2222222222222222222222222222222222222222');

    const exec: EvmCall = {
      type: 'evm_call',
      to: { ref: 'contracts.router' },
      abi: {
        type: 'function',
        name: 'exactInputSingle',
        inputs: [
          {
            name: 'params',
            type: 'tuple',
            components: [
              { name: 'tokenIn', type: 'address' },
              { name: 'amountIn', type: 'uint256' },
            ],
          },
        ],
        outputs: [],
      },
      args: {
        params: {
          object: {
            tokenIn: { ref: 'inputs.tokenIn' },
            amountIn: { lit: '2' },
          },
        },
      },
    };

    const compiled = compileEvmCall(exec, ctx, { chain: 'eip155:1' });
    expect(compiled.chainId).toBe(1);
    expect(compiled.to).toBe('0x1111111111111111111111111111111111111111');
    expect(compiled.data.startsWith('0x')).toBe(true);
    expect(compiled.data.length).toBeGreaterThan(10);
    expect(compiled.value).toBe(0n);
  });

  it('compiles evm_read', () => {
    const ctx = createContext();
    setRef(ctx, 'contracts.c', '0x1111111111111111111111111111111111111111');

    const exec: EvmRead = {
      type: 'evm_read',
      to: { ref: 'contracts.c' },
      abi: {
        type: 'function',
        name: 'balanceOf',
        inputs: [{ name: 'account', type: 'address' }],
        outputs: [{ name: 'balance', type: 'uint256' }],
      },
      args: {
        account: { lit: '0x2222222222222222222222222222222222222222' },
      },
    };

    const compiled = compileEvmRead(exec, ctx, { chain: 'eip155:8453' });
    expect(compiled.chainId).toBe(8453);
    expect(compiled.value).toBe(0n);
  });

  it('fails on invalid to address', () => {
    const ctx = createContext();
    setRef(ctx, 'contracts.router', 'not-an-address');

    const exec: EvmCall = {
      type: 'evm_call',
      to: { ref: 'contracts.router' },
      abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
      args: {},
    };

    expect(() => compileEvmCall(exec, ctx, { chain: 'eip155:1' })).toThrow(EvmCompileError);
  });
});

