import { describe, it, expect } from 'vitest';
import {
  createContext,
  parseProtocolSpec,
  parseWorkflow,
  registerProtocol,
  buildWorkflowExecutionPlan,
  runPlan,
  solver,
  EvmJsonRpcExecutor,
  SolanaRpcExecutor,
  type JsonRpcTransport,
  type EvmSigner,
  type SolanaRpcConnectionLike,
  type SolanaSigner,
} from '../src/index.js';
import { Keypair, PublicKey, Transaction, VersionedTransaction } from '@solana/web3.js';

function u256WordHex(n: bigint): string {
  return `0x${n.toString(16).padStart(64, '0')}`;
}

describe('T422 bridge reference (send → wait_arrival → deposit)', () => {
  it('runs an EVM action, polls a Solana read until satisfied, then sends a Solana instruction', async () => {
    const ctx = createContext();

    const bridgeSpec = parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: bridge-demo, version: "0.0.2" }
deployments:
  - chain: "eip155:1"
    contracts: { bridge: "0x1111111111111111111111111111111111111111" }
actions:
  send:
    description: "send"
    risk_level: 3
    params:
      - { name: amount, type: uint256, description: "amount" }
      - { name: recipient, type: address, description: "recipient" }
    execution:
      "eip155:*":
        type: evm_call
        to: { ref: "contracts.bridge" }
        abi: { type: "function", name: "send", inputs: [{ name: "amount", type: "uint256" }, { name: "recipient", type: "address" }], outputs: [] }
        args:
          amount: { ref: "params.amount" }
          recipient: { ref: "params.recipient" }
`);

    const solRpcSpec = parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: solana-rpc, version: "0.0.2" }
deployments:
  - chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    contracts: {}
queries:
  balance:
    description: "balance"
    params:
      - { name: address, type: address, description: "pubkey" }
    execution:
      "solana:*":
        type: solana_read
        method: "getBalance"
        params:
          object:
            address: { ref: "params.address" }
actions: {}
`);

    const vaultSpec = parseProtocolSpec(`
schema: "ais/0.0.2"
meta: { protocol: solana-vault-demo, version: "0.0.2" }
deployments:
  - chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    contracts: { vault_program: "BPFLoaderUpgradeab1e11111111111111111111111" }
actions:
  deposit:
    description: "deposit"
    risk_level: 2
    params:
      - { name: user, type: address, description: "user" }
    execution:
      "solana:*":
        type: solana_instruction
        program: { ref: "contracts.vault_program" }
        instruction: "deposit"
        accounts:
          - { name: payer, pubkey: { ref: "ctx.wallet_address" }, signer: { lit: true }, writable: { lit: true } }
          - { name: user, pubkey: { ref: "params.user" }, signer: { lit: false }, writable: { lit: false } }
        data: { lit: "0x01" }
`);

    registerProtocol(ctx, bridgeSpec);
    registerProtocol(ctx, solRpcSpec);
    registerProtocol(ctx, vaultSpec);

    const solKeypair = Keypair.generate();
    ctx.runtime.ctx.wallet_address = solKeypair.publicKey.toBase58();

    const wf = parseWorkflow(`
schema: "ais-flow/0.0.2"
meta: { name: demo, version: "0.0.2" }
default_chain: "eip155:1"
inputs:
  evm_amount: { type: uint256, required: true }
  solana_address: { type: address, required: true }
nodes:
  - id: bridge_send
    type: action_ref
    skill: "bridge-demo@0.0.2"
    action: "send"
    args:
      amount: { ref: "inputs.evm_amount" }
      recipient: { lit: "0x2222222222222222222222222222222222222222" }
  - id: wait_arrival
    chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    type: query_ref
    skill: "solana-rpc@0.0.2"
    query: "balance"
    deps: ["bridge_send"]
    args:
      address: { ref: "inputs.solana_address" }
    retry: { interval_ms: 5, max_attempts: 5 }
    until: { cel: "nodes.wait_arrival.outputs.lamports > 1" }
  - id: solana_deposit
    chain: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
    type: action_ref
    skill: "solana-vault-demo@0.0.2"
    action: "deposit"
    deps: ["wait_arrival"]
    args:
      user: { ref: "inputs.solana_address" }
`);

    ctx.runtime.inputs.evm_amount = 1n;
    ctx.runtime.inputs.solana_address = solKeypair.publicKey.toBase58();

    const plan = buildWorkflowExecutionPlan(wf, ctx);

    const transport: JsonRpcTransport = {
      async request(method) {
        if (method === 'eth_sendRawTransaction') return `0x${'ab'.repeat(32)}`;
        if (method === 'eth_call') return u256WordHex(0n);
        if (method === 'eth_getTransactionReceipt') return { status: '0x1' };
        throw new Error(`Unsupported EVM method: ${method}`);
      },
    };
    const evmSigner: EvmSigner = {
      async signTransaction(tx) {
        return `0xdeadbeef${tx.chainId.toString(16)}`;
      },
    };
    const evmExecutor = new EvmJsonRpcExecutor({ transport, signer: evmSigner, wait_for_receipt: false });

    let balanceCalls = 0;
    const connection: SolanaRpcConnectionLike = {
      async getLatestBlockhash() {
        return { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 123 };
      },
      async sendRawTransaction() {
        return '5Y9xWw4zZ4Q1qzQmHqj2dDq7x1o8n9w8k5v8f3v4m3n2n1p1q1r1s1t1u1v1w1x1y1z';
      },
      async confirmTransaction() {
        return { value: { err: null } };
      },
      async getBalance() {
        balanceCalls++;
        return balanceCalls >= 3 ? 2 : 0;
      },
      async getTokenAccountBalance() {
        throw new Error('not used');
      },
      async getAccountInfo() {
        return null;
      },
      async getSignatureStatuses() {
        return { value: [] };
      },
    };
    const solSigner: SolanaSigner = {
      publicKey: solKeypair.publicKey,
      signTransaction(tx) {
        if (tx instanceof Transaction) {
          tx.partialSign(solKeypair);
          return tx;
        }
        if (tx instanceof VersionedTransaction) {
          tx.sign([solKeypair]);
          return tx;
        }
        return tx as any;
      },
    };
    const solExecutor = new SolanaRpcExecutor({
      connection,
      signer: solSigner,
      wait_for_confirmation: false,
    });

    const events: any[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [evmExecutor, solExecutor] })) {
      events.push(ev);
    }

    expect(events.some((e) => e.type === 'tx_sent')).toBe(true); // EVM + Solana both emit tx_sent
    expect(events.some((e) => e.type === 'node_waiting' && e.node?.id === 'wait_arrival')).toBe(true);
    expect(balanceCalls).toBeGreaterThanOrEqual(3);
    expect(ctx.runtime.nodes.wait_arrival?.outputs?.lamports).toBe(2n);
    expect(typeof ctx.runtime.nodes.bridge_send?.outputs?.tx_hash).toBe('string');
    expect(typeof ctx.runtime.nodes.solana_deposit?.outputs?.signature).toBe('string');
  });
});
