import { describe, it, expect } from 'vitest';
import { createContext, registerProtocol, parseProtocolSpec, parseWorkflow, buildWorkflowExecutionPlan } from '../src/index.js';

describe('composite → plan expansion', () => {
  it('expands composite action into ordered plan nodes and preserves parent node id as last step', () => {
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
  do:
    description: "composite write"
    risk_level: 1
    params: [{ name: x, type: uint256, description: "x" }]
    execution:
      "eip155:*":
        type: composite
        steps:
          - id: approve
            description: "approve"
            execution:
              type: evm_call
              to: { ref: "contracts.router" }
              abi: { type: "function", name: "approve", inputs: [{ name: "x", type: "uint256" }], outputs: [] }
              args: { x: { ref: "params.x" } }
          - id: swap
            description: "swap"
            execution:
              type: evm_call
              to: { ref: "contracts.router" }
              abi: { type: "function", name: "swap", inputs: [{ name: "x", type: "uint256" }], outputs: [] }
              args: { x: { ref: "params.x" } }
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: wf, version: "0.0.2" }
default_chain: "eip155:1"
inputs: { amount: { type: uint256 } }
nodes:
  - id: n1
    type: action_ref
    skill: "demo@0.0.2"
    action: do
    args:
      x: { ref: "inputs.amount" }
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    expect(plan.nodes.map((n) => n.id)).toEqual(['n1__approve', 'n1']);
    expect(plan.nodes[0]!.kind).toBe('execution');
    expect(plan.nodes[0]!.execution.type).toBe('evm_call');
    expect(plan.nodes[1]!.kind).toBe('execution');
    expect(plan.nodes[1]!.execution.type).toBe('evm_call');
    expect(plan.nodes[1]!.deps).toEqual(['n1__approve']);

    // Step outputs are written under nodes.<parent>.outputs.steps.<stepId>
    expect(plan.nodes[0]!.writes).toEqual([{ path: 'nodes.n1.outputs.steps.approve', mode: 'set' }]);
    expect(plan.nodes[1]!.writes).toEqual([
      { path: 'nodes.n1.outputs', mode: 'merge' },
      { path: 'nodes.n1.outputs.steps.swap', mode: 'set' },
    ]);
  });

  it('supports cross-chain composite steps via steps[].chain', () => {
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
  bridge_like:
    description: "cross-chain"
    risk_level: 1
    execution:
      "eip155:*":
        type: composite
        steps:
          - id: evm_send
            chain: "eip155:1"
            execution:
              type: evm_call
              to: { ref: "contracts.router" }
              abi: { type: "function", name: "send", inputs: [], outputs: [] }
              args: {}
          - id: solana_deposit
            chain: "solana:mainnet"
            execution:
              type: solana_instruction
              program: { lit: "11111111111111111111111111111111" }
              instruction: "deposit"
              accounts:
                - name: payer
                  pubkey: { lit: "11111111111111111111111111111111" }
                  signer: { lit: true }
                  writable: { lit: true }
              data: { lit: "0x" }
`)
    );

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: wf, version: "0.0.2" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: action_ref
    skill: "demo@0.0.2"
    action: bridge_like
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    expect(plan.nodes.map((n) => [n.id, n.chain, n.execution.type])).toEqual([
      ['n1__evm_send', 'eip155:1', 'evm_call'],
      ['n1', 'solana:mainnet', 'solana_instruction'],
    ]);
    expect(plan.nodes[1]!.deps).toEqual(['n1__evm_send']);
  });
});

