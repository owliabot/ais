import test from 'node:test';
import assert from 'node:assert/strict';

import { dryRunCompilePlanJson } from '../dist/dry-run.js';

test('dryRunCompilePlanJson returns json payload', async () => {
  const sdk = {
    createSolver: () => ({ solve: async () => ({ patches: [] }) }),
    solver: { solve: async () => ({ patches: [] }) },
    getNodeReadiness: () => ({ state: 'ready', resolved_params: {} }),
    applyRuntimePatches: () => {},
    compileEvmExecution: () => ({ kind: 'evm_call', chain: 'eip155:1', chainId: 1, to: '0x' + '11'.repeat(20), value: 0, data: '0x', abi: { name: 'x' } }),
    solana: {},
    compileWritePreview: () => ({ kind: 'noop' }),
    extractPolicyGateInput: () => ({ chain: 'eip155:1' }),
    enforcePolicyGate: () => ({ kind: 'ok' }),
    explainPolicyGateResult: (r) => r,
    resolveAction: () => ({ action: { risk_level: 1, risk_tags: [] } }),
    ExecutionPlanSchema: { safeParse: () => ({ success: true, data: null }) },
    StructuredIssueSchema: { safeParse: () => ({ success: false }) },
    fromZodError: () => [],
    fromPlanBuildError: () => [],
  };

  const plan = {
    schema: 'ais-plan/0.0.3',
    meta: { name: 'x', description: 'y', created_at: '2020-01-01T00:00:00.000Z', extensions: {} },
    nodes: [
      {
        id: 'n1',
        chain: 'eip155:1',
        kind: 'execution',
        execution: { type: 'evm_call', to: { lit: '0x' + '11'.repeat(20) }, abi: { type: 'function', name: 'x', inputs: [], outputs: [] }, args: {} },
        extensions: {},
      },
    ],
    extensions: {},
  };

  const payload = await dryRunCompilePlanJson({ sdk, plan, ctx: { runtime: { ctx: {}, inputs: {}, refs: {}, nodes: {} } } });
  assert.equal(payload.kind, 'dry_run_compile_plan');
  assert.ok(Array.isArray(payload.nodes));
  assert.equal(payload.nodes[0].id, 'n1');
});
