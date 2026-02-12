import { describe, it, expect } from 'vitest';
import {
  applyRuntimePatch,
  applyRuntimePatches,
  createContext,
  validateRuntimePatch,
  checkRuntimePatchPathAllowed,
} from '../src/index.js';

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
    expect(r.audit.patch_count).toBe(2);
    expect(r.audit.rejected_count).toBe(0);
    expect(r.audit.hash.length).toBe(64);
    expect(ctx.runtime.ctx.sender).toBe('0x' + '11'.repeat(20));
    expect(ctx.runtime.inputs.amount).toBe('2.0');
  });

  it('RuntimePatch schema accepts extensions and rejects unknown fields', () => {
    const ok = validateRuntimePatch({
      op: 'set',
      path: 'ctx.foo',
      value: 1,
      extensions: { source: 'agent' },
    });
    expect(ok.ok).toBe(true);

    const bad = validateRuntimePatch({
      op: 'set',
      path: 'ctx.foo',
      value: 1,
      extra: true,
    });
    expect(bad.ok).toBe(false);
  });

  it('guard defaults allow inputs/ctx/contracts/policy and block nodes', () => {
    expect(checkRuntimePatchPathAllowed('inputs.amount').ok).toBe(true);
    expect(checkRuntimePatchPathAllowed('ctx.now').ok).toBe(true);
    expect(checkRuntimePatchPathAllowed('contracts.router').ok).toBe(true);
    expect(checkRuntimePatchPathAllowed('policy.approval').ok).toBe(true);
    expect(checkRuntimePatchPathAllowed('nodes.n1.outputs').ok).toBe(false);
  });

  it('applyRuntimePatches rejects unauthorized paths with structured error when guard enabled', () => {
    const ctx = createContext();
    expect(() =>
      applyRuntimePatches(
        ctx,
        [{ op: 'set', path: 'nodes.n1.outputs', value: { ok: true } }],
        { guard: { enabled: true } }
      )
    ).toThrow(/guard/i);
  });

  it('guard can allow nodes paths via regex allow_nodes_paths', () => {
    const ctx = createContext();
    const result = applyRuntimePatches(
      ctx,
      [{ op: 'set', path: 'nodes.n1.outputs.quote', value: 1 }],
      {
        guard: {
          enabled: true,
          policy: {
            allow_nodes_paths: ['^nodes\\.[a-zA-Z0-9_-]+\\.outputs\\..+$'],
          },
        },
      }
    );
    expect(result.applied_count).toBe(1);
    expect((ctx.runtime.nodes as any).n1.outputs.quote).toBe(1);
  });
});
