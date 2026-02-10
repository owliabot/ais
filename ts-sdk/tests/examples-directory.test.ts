import { describe, it, expect } from 'vitest';
import { resolve } from 'node:path';
import { loadDirectoryAsContext, validateWorkflow } from '../src/index.js';

describe('examples/ directory', () => {
  it('loads all examples and validates key workflows', async () => {
    const examplesDir = resolve(process.cwd(), '..', 'examples');
    const { context, result } = await loadDirectoryAsContext(examplesDir, { recursive: true });

    expect(result.errors).toEqual([]);
    expect(result.protocols.length).toBeGreaterThan(0);
    expect(result.workflows.length).toBeGreaterThan(0);

    const wf = result.workflows.find((w) => w.document.meta.name === 'aave-branch-bridge-solana-deposit');
    expect(wf).toBeTruthy();

    const validated = validateWorkflow(wf!.document, context);
    expect(validated.valid).toBe(true);
  });
});
