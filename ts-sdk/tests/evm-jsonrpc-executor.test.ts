import { describe, it, expect } from 'vitest';
import {
  createContext,
  setRef,
  parseProtocolSpec,
  registerProtocol,
  parseWorkflow,
  buildWorkflowExecutionPlan,
  EvmJsonRpcExecutor,
  type JsonRpcTransport,
  type EvmSigner,
} from '../src/index.js';

describe('EvmJsonRpcExecutor (reference)', () => {
  it('executes evm_read via eth_call and writes outputs to nodes.<id>.outputs', async () => {
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
meta: { name: demo, version: "0.0.2" }
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

    setRef(ctx, 'inputs.amount', 7n);
    setRef(ctx, 'contracts.router', '0x1111111111111111111111111111111111111111');

    const plan = buildWorkflowExecutionPlan(wf, ctx);
    const node = plan.nodes[0]!;

    const transport: JsonRpcTransport = {
      async request(method, params) {
        expect(method).toBe('eth_call');
        expect(params[0]).toEqual({
          to: '0x1111111111111111111111111111111111111111',
          data: expect.stringMatching(/^0x/),
        });
        // Return uint256(y)=5 encoded as 32-byte word
        return '0x' + '0'.repeat(63) + '5';
      },
    };

    const executor = new EvmJsonRpcExecutor({ transport });
    const result = await executor.execute(node, ctx);
    expect(result.outputs).toEqual({ y: 5n });
    expect(result.patches?.[0]).toEqual({
      op: 'set',
      path: 'nodes.q1.outputs',
      value: { y: 5n },
    });
  });

  it('executes evm_rpc eth_getBalance and writes {balance} output', async () => {
    const ctx = createContext();
    const node: any = {
      id: 'q1',
      chain: 'eip155:1',
      kind: 'query_ref',
      execution: {
        type: 'evm_rpc',
        method: 'eth_getBalance',
        params: { lit: ['0x1111111111111111111111111111111111111111', 'latest'] },
      },
      bindings: {},
      writes: [],
    };

    const transport: JsonRpcTransport = {
      async request(method, params) {
        expect(method).toBe('eth_getBalance');
        expect(params).toEqual(['0x1111111111111111111111111111111111111111', 'latest']);
        return '0x5';
      },
    };

    const executor = new EvmJsonRpcExecutor({ transport });
    const result = await executor.execute(node, ctx);
    expect(result.outputs).toEqual({ balance: 5n });
    expect(result.patches?.[0]).toEqual({
      op: 'merge',
      path: 'nodes.q1.outputs',
      value: { balance: 5n },
    });
  });

  it('returns need_user_confirm for evm_call when signer is missing', async () => {
    const ctx = createContext();
    const node: any = {
      id: 'a1',
      chain: 'eip155:1',
      kind: 'action_ref',
      execution: {
        type: 'evm_call',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
        args: {},
      },
      bindings: {},
      writes: [],
    };

    const transport: JsonRpcTransport = { async request() { throw new Error('not used'); } };
    const executor = new EvmJsonRpcExecutor({ transport });
    const r = await executor.execute(node, ctx);
    expect(r.need_user_confirm?.reason).toContain('Missing signer');
  });

  it('signs and sends evm_call and merges tx_hash into nodes.<id>.outputs', async () => {
    const ctx = createContext();
    const node: any = {
      id: 'a1',
      chain: 'eip155:1',
      kind: 'action_ref',
      execution: {
        type: 'evm_call',
        to: { lit: '0x1111111111111111111111111111111111111111' },
        abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
        args: {},
        value: { lit: '0' },
      },
      bindings: {},
      writes: [],
    };

    const calls: Array<{ method: string; params: unknown[] }> = [];
    const transport: JsonRpcTransport = {
      async request(method, params) {
        calls.push({ method, params });
        if (method === 'eth_sendRawTransaction') return '0x' + 'ab'.repeat(32);
        throw new Error(`Unexpected method: ${method}`);
      },
    };

    const signer: EvmSigner = {
      async signTransaction(tx) {
        expect(tx.to).toBe('0x1111111111111111111111111111111111111111');
        expect(tx.chainId).toBe(1);
        return '0xdeadbeef';
      },
    };

    const executor = new EvmJsonRpcExecutor({ transport, signer });
    const r = await executor.execute(node, ctx);

    expect(calls[0]?.method).toBe('eth_sendRawTransaction');
    expect(r.outputs).toEqual({ tx_hash: '0x' + 'ab'.repeat(32) });
    expect(r.patches?.[0]).toEqual({
      op: 'merge',
      path: 'nodes.a1.outputs',
      value: { tx_hash: '0x' + 'ab'.repeat(32) },
    });
  });
});
