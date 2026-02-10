import { describe, it, expect } from 'vitest';
import {
  createContext,
  runPlan,
  type ExecutionPlan,
  type Executor,
  type Solver,
  SolanaRpcExecutor,
  type SolanaRpcConnectionLike,
} from '../src/index.js';
import { Keypair, PublicKey, Transaction, VersionedTransaction } from '@solana/web3.js';

function lit<T>(v: T) {
  return { lit: v } as any;
}

describe('SolanaRpcExecutor', () => {
  it('executes solana_read getBalance and writes outputs to nodes.<id>.outputs', async () => {
    const ctx = createContext();
    const payer = Keypair.generate();

    const connection: SolanaRpcConnectionLike = {
      async getLatestBlockhash() {
        return { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 123 };
      },
      async sendRawTransaction() {
        throw new Error('not used');
      },
      async confirmTransaction() {
        throw new Error('not used');
      },
      async getBalance() {
        return 7;
      },
      async getTokenAccountBalance() {
        throw new Error('not used');
      },
      async getAccountInfo() {
        throw new Error('not used');
      },
      async getSignatureStatuses() {
        throw new Error('not used');
      },
    };

    const executor = new SolanaRpcExecutor({
      connection,
      signer: {
        publicKey: payer.publicKey,
        signTransaction(tx) {
          return tx as any;
        },
      },
      wait_for_confirmation: false,
    });

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'q1',
          chain: 'solana:mainnet',
          kind: 'execution',
          execution: {
            type: 'solana_read',
            method: 'getBalance',
            params: { object: { address: { lit: payer.publicKey.toBase58() } } },
          } as any,
          writes: [{ path: 'nodes.q1.outputs', mode: 'set' }],
        },
      ],
    };

    const solver: Solver = { solve() { return {}; } };
    const events: any[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor as unknown as Executor] })) {
      events.push(ev);
    }

    expect(ctx.runtime.nodes.q1?.outputs).toEqual({ lamports: 7n });
    expect(events.some((e) => e.type === 'query_result' && e.node.id === 'q1')).toBe(true);
  });

  it('polls solana_read with until/retry until satisfied', async () => {
    const ctx = createContext();
    const payer = Keypair.generate();

    let calls = 0;
    const connection: SolanaRpcConnectionLike = {
      async getLatestBlockhash() {
        return { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 123 };
      },
      async sendRawTransaction() {
        throw new Error('not used');
      },
      async confirmTransaction() {
        throw new Error('not used');
      },
      async getBalance() {
        calls++;
        return calls >= 2 ? 1 : 0;
      },
      async getTokenAccountBalance() {
        throw new Error('not used');
      },
      async getAccountInfo() {
        throw new Error('not used');
      },
      async getSignatureStatuses() {
        throw new Error('not used');
      },
    };

    const executor = new SolanaRpcExecutor({ connection, wait_for_confirmation: false, fee_payer: payer.publicKey });

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'q1',
          chain: 'solana:mainnet',
          kind: 'execution',
          execution: {
            type: 'solana_read',
            method: 'getBalance',
            params: { object: { address: { lit: payer.publicKey.toBase58() } } },
          } as any,
          writes: [{ path: 'nodes.q1.outputs', mode: 'set' }],
          until: { cel: 'nodes.q1.outputs.lamports > 0' } as any,
          retry: { interval_ms: 5, max_attempts: 5 } as any,
        },
      ],
    };

    const solver: Solver = { solve() { return {}; } };
    const events: any[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor as unknown as Executor] })) {
      events.push(ev);
    }

    expect(events.some((e) => e.type === 'node_waiting' && e.node.id === 'q1')).toBe(true);
    expect(ctx.runtime.nodes.q1?.outputs?.lamports).toBe(1n);
  });

  it('signs, sends, confirms, and writes outputs.signature', async () => {
    const ctx = createContext();
    const payer = Keypair.generate();

    let sentRaw: Uint8Array | null = null;
    let confirmed = false;

    const connection: SolanaRpcConnectionLike = {
      async getLatestBlockhash() {
        return { blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 123 };
      },
      async sendRawTransaction(raw) {
        sentRaw = raw;
        return '5Y9xWw4zZ4Q1qzQmHqj2dDq7x1o8n9w8k5v8f3v4m3n2n1p1q1r1s1t1u1v1w1x1y1z';
      },
      async confirmTransaction() {
        confirmed = true;
        return { value: { err: null } };
      },
    };

    const executor = new SolanaRpcExecutor({
      connection,
      signer: {
        publicKey: payer.publicKey,
        signTransaction(tx) {
          if (tx instanceof Transaction) {
            tx.partialSign(payer);
            return tx;
          }
          if (tx instanceof VersionedTransaction) {
            tx.sign([payer]);
            return tx;
          }
          return tx as any;
        },
      },
      wait_for_confirmation: true,
    });

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 's1',
          chain: 'solana:mainnet',
          kind: 'execution',
          execution: {
            type: 'solana_instruction',
            program: { lit: new PublicKey('BPFLoaderUpgradeab1e11111111111111111111111').toBase58() },
            instruction: 'custom',
            accounts: [
              { name: 'a', pubkey: { lit: payer.publicKey.toBase58() }, signer: { lit: false }, writable: { lit: false } },
            ],
            data: { lit: '0x01' },
          } as any,
        },
      ],
    };

    const solver: Solver = { solve() { return {}; } };
    const events: any[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor as unknown as Executor] })) {
      events.push(ev);
    }

    expect(sentRaw).toBeInstanceOf(Uint8Array);
    expect(confirmed).toBe(true);
    expect(ctx.runtime.nodes.s1?.outputs?.signature).toBeTruthy();
    expect(events.some((e) => e.type === 'tx_sent' && e.node.id === 's1')).toBe(true);
    expect(events.some((e) => e.type === 'tx_confirmed' && e.node.id === 's1')).toBe(true);
  });
});
