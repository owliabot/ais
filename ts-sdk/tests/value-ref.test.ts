import { describe, it, expect } from 'vitest';
import {
  createContext,
  setRef,
  evaluateValueRef,
  evaluateValueRefAsync,
  ValueRefEvalError,
  type DetectResolver,
} from '../src/index.js';

describe('ValueRef evaluation', () => {
  it('evaluates lit/ref/object/array', () => {
    const ctx = createContext();
    setRef(ctx, 'inputs.amount', 1000);

    expect(evaluateValueRef({ lit: 'x' }, ctx)).toBe('x');
    expect(evaluateValueRef({ ref: 'inputs.amount' }, ctx)).toBe(1000);

    expect(
      evaluateValueRef(
        { object: { a: { ref: 'inputs.amount' }, b: { lit: 1 } } },
        ctx
      )
    ).toEqual({ a: 1000, b: 1 });

    expect(evaluateValueRef({ array: [{ lit: 1 }, { ref: 'inputs.amount' }] }, ctx)).toEqual([
      1,
      1000,
    ]);
  });

  it('throws on missing ref', () => {
    const ctx = createContext();
    expect(() => evaluateValueRef({ ref: 'inputs.missing' }, ctx)).toThrow(ValueRefEvalError);
  });

  it('evaluates cel against runtime root', () => {
    const ctx = createContext();
    setRef(ctx, 'inputs.amount', 3);
    expect(evaluateValueRef({ cel: 'inputs.amount + 2' }, ctx)).toBe(5n);
  });

  it('supports detect.choose_one with candidates', () => {
    const ctx = createContext();
    expect(
      evaluateValueRef(
        {
          detect: {
            kind: 'choose_one',
            candidates: [{ lit: 'a' }, { lit: 'b' }],
          },
        },
        ctx
      )
    ).toBe('a');
  });

  it('requires a detect resolver for non-choose_one kinds', async () => {
    const ctx = createContext();
    await expect(
      evaluateValueRefAsync({ detect: { kind: 'best_quote' } }, ctx)
    ).rejects.toThrow(ValueRefEvalError);
  });

  it('supports async detect via evaluateValueRefAsync', async () => {
    const ctx = createContext();
    const detect: DetectResolver = {
      resolve: async () => 'ok',
    };
    await expect(
      evaluateValueRefAsync({ detect: { kind: 'best_quote', provider: 'x' } }, ctx, { detect })
    ).resolves.toBe('ok');
  });
});
