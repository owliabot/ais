import { describe, it, expect } from 'vitest';
import {
  createContext,
  registerProtocol,
  parseProtocolSpec,
  parseWorkflow,
  setRef,
  buildWorkflowExecutionPlan,
  ExecutionPlanSchema,
  getNodeReadiness,
} from '../src/index.js';

describe('ExecutionPlan IR (ais-plan/0.0.3)', () => {
  it('builds a JSON-serializable workflow plan and validates schema', () => {
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
actions:
  a:
    description: "write"
    risk_level: 1
    params: [{ name: x, type: uint256, description: "x" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "a", inputs: [{ name: "x", type: "uint256" }], outputs: [] }
        args: { x: { ref: "params.x" } }
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: demo, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: n1
    type: query_ref
    protocol: "demo@0.0.2"
    query: q
    args:
      x: { ref: "inputs.amount" }
  - id: n2
    type: action_ref
    protocol: "demo@0.0.2"
    action: a
    args:
      x: { ref: "inputs.amount" }
    deps: ["n1"]
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    expect(plan.schema).toBe('ais-plan/0.0.3');
    expect(plan.nodes).toHaveLength(2);

    // JSON roundtrip + schema validation
    const json = JSON.stringify(plan);
    const parsed = ExecutionPlanSchema.parse(JSON.parse(json));
    expect(parsed.nodes[0].id).toBe('n1');
  });

  it('toposorts workflow nodes using deps + inferred refs', () => {
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
actions:
  a:
    description: "write"
    risk_level: 1
    params: [{ name: x, type: uint256, description: "x" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "a", inputs: [{ name: "x", type: "uint256" }], outputs: [] }
        args: { x: { ref: "params.x" } }
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: demo, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: n2
    type: action_ref
    protocol: "demo@0.0.2"
    action: a
    args:
      x: { ref: "nodes.n1.outputs.y" }
  - id: n1
    type: query_ref
    protocol: "demo@0.0.2"
    query: q
    args:
      x: { ref: "inputs.amount" }
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    expect(plan.nodes.map((n) => n.id)).toEqual(['n1', 'n2']);
    expect(plan.nodes[1]?.deps).toContain('n1');
  });

  it('computes readiness and reports missing refs', () => {
    const ctx = createContext();
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
actions:
  a:
    description: "write"
    risk_level: 1
    params: [{ name: x, type: uint256, description: "x" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "a", inputs: [{ name: "x", type: "uint256" }], outputs: [] }
        args: { x: { ref: "params.x" } }
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.3"
meta: { name: demo, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: n2
    type: action_ref
    protocol: "demo@0.0.2"
    action: a
    args:
      x: { ref: "inputs.amount" }
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    const node = plan.nodes[0]!;

    // missing inputs.amount + contracts.router
    const r1 = getNodeReadiness(node, ctx);
    expect(r1.state).toBe('blocked');
    expect(r1.missing_refs).toContain('inputs.amount');
    expect(r1.missing_refs).toContain('contracts.router');

    setRef(ctx, 'inputs.amount', 7n);
    setRef(ctx, 'contracts.router', '0x1111111111111111111111111111111111111111');
    const r2 = getNodeReadiness(node, ctx);
    expect(r2.state).toBe('ready');
    expect(r2.resolved_params?.x).toBe(7n);
  });
});
