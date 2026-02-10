import { describe, it, expect } from 'vitest';
import { applyRuntimePatch, applyRuntimePatches, createContext } from '../src/index.js';

describe('engine RuntimePatch', () => {
  it('applies set patches and records undo', () => {
    const ctx = createContext();
    const r = applyRuntimePatch(
      ctx,
      { op: 'set', path: 'inputs.amount', value: '1.0' },
      { record_undo: true }
    );
    expect(r.undo).toEqual([{ op: 'set', path: 'inputs.amount', value: undefined }]);
    expect(ctx.runtime.inputs.amount).toBe('1.0');
  });

  it('applies merge patches (shallow) and errors on non-object target', () => {
    const ctx = createContext();
    ctx.runtime.nodes.n1 = { outputs: { a: 1 } };

    applyRuntimePatch(ctx, { op: 'merge', path: 'nodes.n1.outputs', value: { b: 2 } });
    expect(ctx.runtime.nodes.n1.outputs).toEqual({ a: 1, b: 2 });

    applyRuntimePatch(ctx, { op: 'set', path: 'nodes.n2.outputs', value: 5 });
    expect(() =>
      applyRuntimePatch(ctx, { op: 'merge', path: 'nodes.n2.outputs', value: { x: 1 } })
    ).toThrow(/Cannot merge into non-object/);
  });

  it('applies multiple patches and returns combined undo', () => {
    const ctx = createContext();
    const r = applyRuntimePatches(
      ctx,
      [
        { op: 'set', path: 'ctx.sender', value: '0x' + '11'.repeat(20) },
        { op: 'set', path: 'inputs.amount', value: '2.0' },
      ],
      { record_undo: true }
    );

    expect(r.undo).toHaveLength(2);
    expect(ctx.runtime.ctx.sender).toBe('0x' + '11'.repeat(20));
    expect(ctx.runtime.inputs.amount).toBe('2.0');
  });
});

