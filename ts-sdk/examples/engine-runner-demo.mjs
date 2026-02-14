import {
  readFile,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

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
} from '../dist/index.js';

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

function u256WordHex(n) {
  const hex = BigInt(n).toString(16).padStart(64, '0');
  return `0x${hex}`;
}

async function main() {
  const __filename = fileURLToPath(import.meta.url);
  const __dirname = path.dirname(__filename);

  const checkpointPath = path.join('/tmp', 'ais-engine-checkpoint.json');
  const checkpointStore = new FileCheckpointStore(checkpointPath);

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
schema: "ais-flow/0.0.3"
meta: { name: demo-flow, version: "0.0.2" }
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
  - id: a1
    type: action_ref
    protocol: "demo@0.0.2"
    action: swap
    deps: ["q1"]
    args:
      amount_in: { ref: "nodes.q1.outputs.y" }
`;

  const ctx = createContext();
  registerProtocol(ctx, parseProtocolSpec(protocolYaml));
  const wf = parseWorkflow(workflowYaml);

  // Intentionally do NOT set contracts.router so the built-in solver can fill it
  ctx.runtime.inputs.amount = 7n;

  const plan = buildWorkflowExecutionPlan(wf, ctx);

  // Mock JSON-RPC transport: eth_call returns y=5; sendRawTransaction returns a tx hash; receipt can be polled.
  let receiptCalls = 0;
  const transport = {
    async request(method, params) {
      if (method === 'eth_call') {
        // Return uint256(y)=5
        return u256WordHex(5n);
      }
      if (method === 'eth_sendRawTransaction') {
        return `0x${'ab'.repeat(32)}`;
      }
      if (method === 'eth_getTransactionReceipt') {
        receiptCalls++;
        if (receiptCalls < 2) return null;
        return { status: '0x1', transactionHash: params?.[0] };
      }
      throw new Error(`Unsupported method in demo transport: ${method}`);
    },
  };

  const signer = {
    async signTransaction(tx) {
      // This is a demo; do NOT use in production.
      return `0xdeadbeef${tx.chainId.toString(16)}`;
    },
  };

  const executor = new EvmJsonRpcExecutor({ transport, signer, wait_for_receipt: true });

  console.log(`Checkpoint file: ${checkpointPath}`);
  console.log(`Plan nodes: ${plan.nodes.map((n) => `${n.id}:${n.execution.type}`).join(', ')}`);
  console.log('--- events ---');

  for await (const ev of runPlan(plan, ctx, {
    solver,
    executors: [executor],
    checkpoint_store: checkpointStore,
    resume_from_checkpoint: false,
    include_events_in_checkpoint: true,
  })) {
    if (ev.type === 'plan_ready') console.log('plan_ready');
    else if (ev.type === 'node_ready') console.log(`node_ready: ${ev.node.id}`);
    else if (ev.type === 'node_blocked') console.log(`node_blocked: ${ev.node.id} missing=${ev.readiness.missing_refs.join(',')}`);
    else if (ev.type === 'solver_applied') console.log(`solver_applied: ${ev.node.id} patches=${ev.patches.length}`);
    else if (ev.type === 'query_result')
      console.log(
        `query_result: ${ev.node.id} outputs=${JSON.stringify(ev.outputs, (_k, v) => (typeof v === 'bigint' ? v.toString() : v))}`
      );
    else if (ev.type === 'tx_sent') console.log(`tx_sent: ${ev.node.id} hash=${ev.tx_hash}`);
    else if (ev.type === 'tx_confirmed') console.log(`tx_confirmed: ${ev.node.id}`);
    else if (ev.type === 'need_user_confirm') {
      console.log(`need_user_confirm: ${ev.node.id} reason=${ev.reason}`);
      break;
    } else if (ev.type === 'error') {
      console.error(`error: ${ev.node?.id ?? 'global'} ${ev.error?.message ?? ev.error}`);
      break;
    } else if (ev.type === 'checkpoint_saved') {
      console.log(`checkpoint_saved: completed=${ev.checkpoint.completed_node_ids.length}`);
    } else {
      console.log(ev.type);
    }
  }

  console.log('--- runtime ---');
  console.log(JSON.stringify(ctx.runtime, (_k, v) => (typeof v === 'bigint' ? v.toString() : v), 2));
}

main().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
