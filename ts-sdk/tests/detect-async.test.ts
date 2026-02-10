import { describe, it, expect } from 'vitest';
import {
  createContext,
  parseProtocolSpec,
  parseWorkflow,
  registerProtocol,
  buildWorkflowExecutionPlan,
  runPlan,
  createDetectProviderRegistry,
  createDetectResolver,
  EvmJsonRpcExecutor,
  type JsonRpcTransport,
  type EvmSigner,
} from '../src/index.js';

function u256WordHex(n: bigint): string {
  return `0x${n.toString(16).padStart(64, '0')}`;
}

describe('T165 detect provider interface', () => {
  it('allows async detect to unblock readiness and run a node', async () => {
    const ctx = createContext();
    registerProtocol(
      ctx,
      parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { target: "0x1111111111111111111111111111111111111111" }
queries:
  q:
    description: "q"
    params: [{ name: x, type: uint256, description: "x" }]
    returns: [{ name: y, type: uint256 }]
    execution:
      "eip155:*":
        type: evm_read
        to: { ref: "contracts.target" }
        abi: { type: "function", name: "q", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions: {}
`)
    );
    // Keep the test focused on async detect behavior (not contract auto-fill).
    ctx.runtime.contracts.target = '0x1111111111111111111111111111111111111111';

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: wf, version: "0.0.2" }
default_chain: "eip155:1"
nodes:
  - id: n1
    type: query_ref
    skill: "demo@0.0.2"
    query: q
    args:
      x:
        detect:
          kind: protocol_specific
          provider: p
          candidates: [{ lit: "42" }]
`);

    const plan = buildWorkflowExecutionPlan(wf, ctx);

    const registry = createDetectProviderRegistry();
    registry.register('protocol_specific', 'p', async (detect) => {
      // simulate async IO
      await new Promise((r) => setTimeout(r, 1));
      const c = detect.candidates?.[0];
      return c ?? '42';
    });
    const detect = createDetectResolver(registry);

    let ethCallCount = 0;
    const transport: JsonRpcTransport = {
      async request(method) {
        if (method === 'eth_call') {
          ethCallCount++;
          return u256WordHex(7n);
        }
        throw new Error(`Unexpected JSON-RPC method: ${method}`);
      },
    };
    const evmSigner: EvmSigner = {
      async signTransaction() {
        throw new Error('not used');
      },
    };
    const evmExecutor = new EvmJsonRpcExecutor({ transport, signer: evmSigner, wait_for_receipt: false });

    const events: any[] = [];
    for await (const ev of runPlan(plan, ctx, {
      solver: { solve: () => ({ patches: [] }) },
      executors: [evmExecutor],
      detect,
    })) {
      events.push(ev);
    }

    expect(events.some((e) => e.type === 'need_user_confirm')).toBe(false);
    expect(ethCallCount).toBe(1);
    expect(ctx.runtime.nodes.n1?.outputs?.y).toBe(7n);
  });
});
