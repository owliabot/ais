import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import {
  buildWorkflowExecutionPlan,
  createContext,
  deserializeCheckpoint,
  EvmJsonRpcExecutor,
  parseProtocolSpec,
  parseWorkflow,
  registerProtocol,
  runPlan,
  serializeCheckpoint,
  solver,
} from '../../../ts-sdk/dist/index.js';

function u256WordHex(n) {
  const hex = BigInt(n).toString(16).padStart(64, '0');
  return `0x${hex}`;
}

class FileCheckpointStore {
  constructor(filePath) {
    this.filePath = filePath;
  }
  async load() {
    try {
      const raw = await readFile(this.filePath, 'utf8');
      return deserializeCheckpoint(raw);
    } catch {
      return null;
    }
  }
  async save(checkpoint) {
    await writeFile(this.filePath, serializeCheckpoint(checkpoint, { pretty: true }));
  }
}

function tmpFile(name) {
  return join(tmpdir(), `ais-runner-it-${process.pid}-${Date.now()}-${name}`);
}

test('EVM read+write emits tx events, writes checkpoint, and resume skips execution', async () => {
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
        abi:
          { type: "function", name: "quote", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions:
  swap:
    description: "write tx"
    risk_level: 1
    risk_tags: []
    params: [{ name: amount_in, type: uint256, description: "amount" }]
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.router" }
        abi: { type: "function", name: "swap", inputs: [{ name: "amount_in", type: "uint256" }], outputs: [] }
        args: { amount_in: { ref: "params.amount_in" } }
`;

  const workflowYaml = `
schema: "ais-flow/0.0.2"
meta: { name: demo-flow, version: "0.0.2" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: q1
    type: query_ref
    skill: "demo@0.0.2"
    query: quote
    args:
      x: { ref: "inputs.amount" }
  - id: a1
    type: action_ref
    skill: "demo@0.0.2"
    action: swap
    deps: ["q1"]
    args:
      amount_in: { ref: "nodes.q1.outputs.y" }
`;

  const ctx = createContext();
  registerProtocol(ctx, parseProtocolSpec(protocolYaml));
  const wf = parseWorkflow(workflowYaml);
  ctx.runtime.inputs.amount = 7n;

  const plan = buildWorkflowExecutionPlan(wf, ctx);

  let receiptCalls = 0;
  const transport = {
    async request(method, params) {
      if (method === 'eth_call') return u256WordHex(5n);
      if (method === 'eth_sendRawTransaction') return `0x${'ab'.repeat(32)}`;
      if (method === 'eth_getTransactionReceipt') {
        receiptCalls++;
        if (receiptCalls < 2) return null;
        return { status: '0x1', transactionHash: params?.[0] };
      }
      throw new Error(`Unsupported method in mock transport: ${method}`);
    },
  };

  const signer = {
    async signTransaction(tx) {
      return `0xdeadbeef${tx.chainId.toString(16)}`;
    },
  };

  const executor = new EvmJsonRpcExecutor({
    transport,
    signer,
    wait_for_receipt: true,
    receipt_poll: { interval_ms: 1, max_attempts: 5 },
  });

  const checkpointPath = tmpFile('checkpoint.json');
  const checkpointStore = new FileCheckpointStore(checkpointPath);

  const evTypes = [];
  for await (const ev of runPlan(plan, ctx, {
    solver,
    executors: [executor],
    checkpoint_store: checkpointStore,
    resume_from_checkpoint: false,
    include_events_in_checkpoint: true,
  })) {
    evTypes.push(ev.type);
  }

  assert.ok(evTypes.includes('query_result'), 'expected query_result event');
  assert.ok(evTypes.includes('tx_sent'), 'expected tx_sent event');
  assert.ok(evTypes.includes('tx_confirmed'), 'expected tx_confirmed event');
  assert.ok(receiptCalls >= 2, 'expected receipt polling (at least 2 calls)');

  const raw = await readFile(checkpointPath, 'utf8');
  const checkpoint = deserializeCheckpoint(raw);
  assert.equal(checkpoint.completed_node_ids.length, 2, 'expected both nodes completed in checkpoint');

  // Resume: should not call transport at all if checkpoint indicates completion.
  const ctx2 = createContext();
  registerProtocol(ctx2, parseProtocolSpec(protocolYaml));

  const transport2 = {
    async request(method) {
      throw new Error(`transport should not be called on resume-complete run; got ${method}`);
    },
  };
  const executor2 = new EvmJsonRpcExecutor({ transport: transport2, signer, wait_for_receipt: true });

  const evTypes2 = [];
  for await (const ev of runPlan(plan, ctx2, {
    solver,
    executors: [executor2],
    checkpoint_store: checkpointStore,
    resume_from_checkpoint: true,
  })) {
    evTypes2.push(ev.type);
  }

  assert.deepEqual(evTypes2, ['plan_ready'], 'expected immediate completion on resume (plan_ready only)');
});

test('EVM read node until/retry polls until condition becomes true', async () => {
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
        abi:
          { type: "function", name: "quote", inputs: [{ name: "x", type: "uint256" }], outputs: [{ name: "y", type: "uint256" }] }
        args: { x: { ref: "params.x" } }
actions: {}
`;

  const workflowYaml = `
schema: "ais-flow/0.0.2"
meta: { name: demo-until, version: "0.0.2" }
default_chain: "eip155:1"
inputs:
  amount: { type: uint256 }
nodes:
  - id: q1
    type: query_ref
    skill: "demo2@0.0.2"
    query: quote
    args:
      x: { ref: "inputs.amount" }
    until: { cel: "nodes.q1.outputs.y == 1" }
    retry: { interval_ms: 1, max_attempts: 5 }
`;

  const ctx = createContext();
  registerProtocol(ctx, parseProtocolSpec(protocolYaml));
  const wf = parseWorkflow(workflowYaml);
  ctx.runtime.inputs.amount = 7n;
  const plan = buildWorkflowExecutionPlan(wf, ctx);

  let calls = 0;
  const transport = {
    async request(method) {
      if (method !== 'eth_call') throw new Error(`unsupported method: ${method}`);
      calls++;
      return calls < 2 ? u256WordHex(0n) : u256WordHex(1n);
    },
  };

  const executor = new EvmJsonRpcExecutor({ transport });

  const evTypes = [];
  for await (const ev of runPlan(plan, ctx, { solver, executors: [executor] })) {
    evTypes.push(ev.type);
  }

  assert.ok(evTypes.includes('node_waiting'), 'expected node_waiting event during until polling');
  const queryResults = evTypes.filter((t) => t === 'query_result').length;
  assert.ok(queryResults >= 2, 'expected query to run at least twice');
  assert.equal(ctx.runtime.nodes.q1.outputs.y, 1n);
});
