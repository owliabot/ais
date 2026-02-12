import { describe, expect, it } from 'vitest';
import {
  buildWorkflowExecutionPlan,
  createContext,
  EvmJsonRpcExecutor,
  parseProtocolSpec,
  parseWorkflow,
  registerProtocol,
  runDeterministicAgentLoop,
  createSolver,
  type Executor,
} from '../src/index.js';

function u256WordHex(n: bigint) {
  const hex = n.toString(16).padStart(64, '0');
  return `0x${hex}`;
}

describe('AGT107 deterministic agent loop', () => {
  it('fills missing inputs via patch and completes', async () => {
    const protocolYaml = `
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2", name: "Demo" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
queries:
  quote:
    description: "read quote"
    params: [{ name: x, type: uint256, description: "x" }]
    returns: [{ name: y, type: uint256 }]
    execution:
      "eip155:*":
        type: evm_read
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "quote", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions: {}
`;

    const workflowYaml = `
schema: "ais-flow/0.0.3"
meta: { name: flow, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: q1
    type: query_ref
    protocol: "demo@0.0.2"
    query: quote
    args:
      x: { ref: "inputs.amount" }
`;

    const ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(protocolYaml));
    const wf = parseWorkflow(workflowYaml);
    const plan = buildWorkflowExecutionPlan(wf, ctx);

    const transport = {
      async request(method: string) {
        if (method === 'eth_call') return u256WordHex(5n);
        throw new Error(`Unsupported method in mock transport: ${method}`);
      },
    };
    const executor = new EvmJsonRpcExecutor({ transport });

    const res = await runDeterministicAgentLoop({
      plan,
      ctx,
      engine: { solver: createSolver({ auto_fill_contracts: true }), executors: [executor] },
      config: { fill: { 'inputs.amount': 7n }, auto_approve: false },
    });

    expect(res.ok).toBe(true);
    expect(ctx.runtime.inputs.amount).toBe(7n);
    expect(ctx.runtime.nodes.q1.outputs.y).toBe(5n);
  });

  it('fills missing contracts via solver patch and completes', async () => {
    const protocolYaml = `
schema: "ais/0.0.2"
meta: { protocol: demo2, version: "0.0.2", name: "Demo2" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
queries:
  quote:
    description: "read quote"
    params: [{ name: x, type: uint256, description: "x" }]
    returns: [{ name: y, type: uint256 }]
    execution:
      "eip155:*":
        type: evm_read
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "quote", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions: {}
`;

    const workflowYaml = `
schema: "ais-flow/0.0.3"
meta: { name: flow2, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: q1
    type: query_ref
    protocol: "demo2@0.0.2"
    query: quote
    args:
      x: { ref: "inputs.amount" }
`;

    const ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(protocolYaml));
    const wf = parseWorkflow(workflowYaml);
    ctx.runtime.inputs.amount = 7n;
    // Make contracts missing at runtime; solver should auto-fill from deployments.
    ctx.runtime.contracts = {};

    const plan = buildWorkflowExecutionPlan(wf, ctx);

    const transport = {
      async request(method: string) {
        if (method === 'eth_call') return u256WordHex(5n);
        throw new Error(`Unsupported method in mock transport: ${method}`);
      },
    };
    const executor = new EvmJsonRpcExecutor({ transport });

    const res = await runDeterministicAgentLoop({
      plan,
      ctx,
      engine: { solver: createSolver({ auto_fill_contracts: true }), executors: [executor] },
      config: { auto_approve: false },
    });

    expect(res.ok).toBe(true);
    expect(typeof ctx.runtime.contracts.router).toBe('string');
  });

  it('approves policy-like need_user_confirm and completes', async () => {
    const protocolYaml = `
schema: "ais/0.0.2"
meta: { protocol: demo3, version: "0.0.2", name: "Demo3" }
deployments:
  - chain: "eip155:1"
    contracts: { router: "0x1111111111111111111111111111111111111111" }
actions:
  swap:
    description: "write tx"
    risk_level: 4
    params: [{ name: amount_in, type: uint256, description: "amount in" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "swap", inputs: [{ name: amount_in, type: uint256 }], outputs: [] }
        args: { amount_in: { ref: "params.amount_in" } }
queries: {}
`;

    const workflowYaml = `
schema: "ais-flow/0.0.3"
meta: { name: flow3, version: "0.0.3" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: a1
    type: action_ref
    protocol: "demo3@0.0.2"
    action: swap
    args:
      amount_in: { ref: "inputs.amount" }
`;

    const ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(protocolYaml));
    const wf = parseWorkflow(workflowYaml);
    ctx.runtime.inputs.amount = 7n;
    const plan = buildWorkflowExecutionPlan(wf, ctx);

    // Executor that requires explicit approval before executing write.
    const executor: Executor = {
      supports(node) {
        return node.execution.type === 'evm_call';
      },
      async execute(node, ctx2) {
        const approvals = (ctx2.runtime.policy as any)?.runner_approvals ?? {};
        const approved = approvals?.[node.id]?.approved === true;
        if (!approved) {
          return {
            need_user_confirm: {
              reason: 'policy approval required',
              details: {
                kind: 'policy_gate',
                node_id: node.id,
                action_ref: `${node.source?.protocol}/${node.source?.action}`,
                hit_reasons: ['policy approval required'],
                gate: { status: 'need_user_confirm', reason: 'policy approval required' },
              },
            },
          };
        }
        return { outputs: { tx_hash: `0x${'ab'.repeat(32)}` } };
      },
    };

    const res = await runDeterministicAgentLoop({
      plan,
      ctx,
      engine: { solver: createSolver({ auto_fill_contracts: true }), executors: [executor] },
      config: { auto_approve: true },
    });

    expect(res.ok).toBe(true);
    expect(ctx.runtime.policy.runner_approvals.a1.approved).toBe(true);
  });
});
