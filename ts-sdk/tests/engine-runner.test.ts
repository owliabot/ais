import { describe, it, expect } from 'vitest';
import {
  createContext,
  runPlan,
  type EngineEvent,
  type ExecutionPlan,
  type ExecutionPlanNode,
  type Solver,
  type Executor,
} from '../src/index.js';

class MemoryCheckpointStore {
  checkpoint: any = null;
  async load() {
    return this.checkpoint;
  }
  async save(checkpoint: any) {
    this.checkpoint = checkpoint;
  }
}

function lit<T>(v: T) {
  return { lit: v } as any;
}

describe('engine.runPlan (reference)', () => {
  it('runs nodes, applies solver patches, and checkpoints', async () => {
    const ctx = createContext();

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'n1',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: lit('0x1111111111111111111111111111111111111111'),
            abi: {
              type: 'function',
              name: 'q',
              inputs: [],
              outputs: [{ name: 'y', type: 'uint256' }],
            },
            args: {},
          },
          writes: [{ path: 'nodes.n1.outputs', mode: 'set' }],
        },
        {
          id: 'n2',
          chain: 'eip155:1',
          kind: 'execution',
          deps: ['n1'],
          execution: {
            type: 'evm_call',
            to: { ref: 'inputs.to' } as any,
            abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
            args: {},
            value: lit('0'),
          },
        },
      ],
    };

    const solver: Solver = {
      solve(_node, readiness, _ctx) {
        if (readiness.state !== 'blocked') return {};
        if (readiness.missing_refs.includes('inputs.to')) {
          return { patches: [{ op: 'set', path: 'inputs.to', value: '0x2222222222222222222222222222222222222222' }] };
        }
        return { cannot_solve: { reason: 'unexpected missing' } };
      },
    };

    const executor: Executor = {
      supports(node: ExecutionPlanNode) {
        return node.chain.startsWith('eip155:') && (node.execution.type === 'evm_read' || node.execution.type === 'evm_call');
      },
      async execute(node: ExecutionPlanNode) {
        if (node.execution.type === 'evm_read') {
          return { outputs: { y: 5n }, patches: [{ op: 'set', path: 'nodes.n1.outputs', value: { y: 5n } }] };
        }
        return { outputs: { tx_hash: '0x' + 'ab'.repeat(32) }, patches: [{ op: 'merge', path: 'nodes.n2.outputs', value: { tx_hash: '0x' + 'ab'.repeat(32) } }] };
      },
    };

    const store = new MemoryCheckpointStore();
    const events: EngineEvent[] = [];

    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], checkpoint_store: store as any })) {
      events.push(ev);
    }

    expect(events.some((e) => e.type === 'plan_ready')).toBe(true);
    expect(events.some((e) => e.type === 'node_ready' && e.node.id === 'n1')).toBe(true);
    expect(events.some((e) => e.type === 'query_result' && e.node.id === 'n1')).toBe(true);
    expect(events.some((e) => e.type === 'node_blocked' && e.node.id === 'n2')).toBe(true);
    expect(events.some((e) => e.type === 'solver_applied' && e.node.id === 'n2')).toBe(true);
    expect(events.some((e) => e.type === 'tx_sent' && e.node.id === 'n2')).toBe(true);
    expect(ctx.runtime.nodes.n1?.outputs).toEqual({ y: 5n });
    expect(ctx.runtime.nodes.n2?.outputs).toEqual({ tx_hash: '0x' + 'ab'.repeat(32) });
    expect(store.checkpoint?.completed_node_ids?.sort()).toEqual(['n1', 'n2']);
  });

  it('pauses on need_user_confirm and can resume', async () => {
    const ctx = createContext();

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'n1',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: lit('0x1111111111111111111111111111111111111111'),
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] },
            args: {},
          },
        },
        {
          id: 'n2',
          chain: 'eip155:1',
          kind: 'execution',
          deps: ['n1'],
          execution: {
            type: 'evm_call',
            to: { ref: 'inputs.to' } as any,
            abi: { type: 'function', name: 'f', inputs: [], outputs: [] },
            args: {},
            value: lit('0'),
          },
        },
      ],
    };

    const store = new MemoryCheckpointStore();

    let allow = false;
    const solver: Solver = {
      solve(_node, readiness) {
        if (readiness.state !== 'blocked') return {};
        if (!allow) return { need_user_confirm: { reason: 'please confirm' } };
        return { patches: [{ op: 'set', path: 'inputs.to', value: '0x2222222222222222222222222222222222222222' }] };
      },
    };

    const executor: Executor = {
      supports() {
        return true;
      },
      async execute(node: ExecutionPlanNode) {
        return { patches: [{ op: 'merge', path: `nodes.${node.id}.outputs`, value: { ok: true } }], outputs: { ok: true } };
      },
    };

    const first: EngineEvent[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], checkpoint_store: store as any })) {
      first.push(ev);
      if (ev.type === 'need_user_confirm') break;
    }
    expect(first.some((e) => e.type === 'need_user_confirm')).toBe(true);
    expect(store.checkpoint?.completed_node_ids).toEqual(['n1']);

    allow = true;
    const second: EngineEvent[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], checkpoint_store: store as any })) {
      second.push(ev);
    }
    expect(ctx.runtime.nodes.n2?.outputs).toEqual({ ok: true });
    expect(store.checkpoint?.completed_node_ids?.sort()).toEqual(['n1', 'n2']);
    expect(second.some((e) => e.type === 'node_ready' && e.node.id === 'n2')).toBe(true);
  });

  it('polls a read node with until/retry until satisfied', async () => {
    const ctx = createContext();
    const store = new MemoryCheckpointStore();

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.2',
      nodes: [
        {
          id: 'q1',
          chain: 'eip155:1',
          kind: 'execution',
          until: { ref: 'nodes.q1.outputs.arrived' } as any,
          retry: { interval_ms: 5, max_attempts: 5 } as any,
          execution: {
            type: 'evm_read',
            to: lit('0x1111111111111111111111111111111111111111'),
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] },
            args: {},
          },
          writes: [{ path: 'nodes.q1.outputs', mode: 'set' }],
        },
      ],
    };

    let calls = 0;
    const executor: Executor = {
      supports() {
        return true;
      },
      async execute() {
        calls++;
        const arrived = calls >= 2;
        return {
          outputs: { arrived },
          patches: [{ op: 'set', path: 'nodes.q1.outputs', value: { arrived } }],
        };
      },
    };

    const solver: Solver = { solve() { return {}; } };
    const events: EngineEvent[] = [];

    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], checkpoint_store: store as any })) {
      events.push(ev);
    }

    expect(events.filter((e) => e.type === 'query_result').length).toBeGreaterThanOrEqual(2);
    expect(events.some((e) => e.type === 'node_waiting' && e.node.id === 'q1')).toBe(true);
    expect(ctx.runtime.nodes.q1?.outputs).toEqual({ arrived: true });
    expect(store.checkpoint?.completed_node_ids?.sort()).toEqual(['q1']);
  });
});
