import { describe, it, expect } from 'vitest';
import {
  createContext,
  parseProtocolSpec,
  registerProtocol,
  parseWorkflow,
  buildWorkflowExecutionPlan,
  getNodeReadiness,
  applyRuntimePatches,
  solver,
  createSolver,
} from '../src/index.js';

describe('solver (built-in)', () => {
  it('auto-fills contracts.* from protocol deployment when missing', () => {
    const ctx = createContext();
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
queries:
  q:
    description: "read"
    params: [{ name: x, type: uint256, description: "x" }]
    returns: [{ name: y, type: uint256 }]
    execution:
      "eip155:*":
        type: evm_read
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "q", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions: {}
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: demo, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: q1
    type: query_ref
    protocol: "demo@0.0.2"
    query: q
    args:
      x: { ref: "inputs.amount" }
`);

    // only set inputs, not contracts
    ctx.runtime.inputs.amount = 7n;

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    const node = plan.nodes[0]!;
    const r1 = getNodeReadiness(node, ctx);
    expect(r1.state).toBe('blocked');
    expect(r1.missing_refs).toContain('contracts.router');

    const s = solver.solve(node, r1, ctx);
    expect(s.patches?.some((p) => p.path === 'contracts')).toBe(true);

    applyRuntimePatches(ctx, s.patches ?? []);
    const r2 = getNodeReadiness(node, ctx);
    expect(r2.state).toBe('ready');
  });

  it('returns need_user_confirm when inputs.* are missing', () => {
    const ctx = createContext();
    const node: any = {
      id: 'n1',
      chain: 'eip155:1',
      kind: 'query_ref',
      execution: {
        type: 'evm_read',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: { type: 'function', name: 'f', inputs: [{ name: 'x', type: 'uint256' }], outputs: [{ name: 'y', type: 'uint256' }] },
        args: { x: { ref: 'inputs.amount' } },
      },
      bindings: { params: { x: { ref: 'inputs.amount' } } },
      writes: [{ path: 'nodes.n1.outputs', mode: 'set' }],
      source: { protocol: 'demo@0.0.2', query: 'f' },
    };

    const readiness = getNodeReadiness(node, ctx);
    expect(readiness.state).toBe('blocked');
    expect(readiness.missing_refs).toContain('inputs.amount');

    const s = solver.solve(node, readiness, ctx);
    expect(s.need_user_confirm?.reason).toContain('missing runtime inputs');
    expect(s.need_user_confirm?.details).toMatchObject({ missing_refs: ['inputs.amount'] });
  });

  it('can disable auto_fill_contracts', () => {
    const ctx = createContext();
    const custom = createSolver({ auto_fill_contracts: false });
    const node: any = {
      id: 'n1',
      chain: 'eip155:1',
      kind: 'query_ref',
      execution: {
        type: 'evm_read',
        to: { ref: 'contracts.router' },
        abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
        args: {},
      },
      bindings: {},
      writes: [{ path: 'nodes.n1.outputs', mode: 'set' }],
      source: { protocol: 'demo@0.0.2', query: 'f' },
    };
    const readiness = getNodeReadiness(node, ctx);
    const s = custom.solve(node, readiness, ctx);
    expect(s.patches ?? []).toHaveLength(0);
  });
});
