import { describe, it, expect } from 'vitest';
import {
  createContext,
  runPlan,
  type EngineEvent,
  type ExecutionPlan,
  type ExecutionPlanNode,
  type Solver,
  type Executor,
  type ExecutionTraceRecord,
  type ExecutionTraceSink,
} from '../src/index.js';

function lit<T>(v: T) {
  return { lit: v } as any;
}

class MemoryTraceSink implements ExecutionTraceSink {
  records: ExecutionTraceRecord[] = [];
  append(record: ExecutionTraceRecord) {
    this.records.push(record);
  }
}

describe('engine trace sink', () => {
  it('emits a root record, node spans, and event records with parent_id branching', async () => {
    const ctx = createContext();

    const plan: ExecutionPlan = {
      schema: 'ais-plan/0.0.3',
      nodes: [
        {
          id: 'q1',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: lit('0x1111111111111111111111111111111111111111'),
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] },
            args: {},
          },
          writes: [{ path: 'nodes.q1.outputs', mode: 'set' }],
        },
        {
          id: 'q2',
          chain: 'eip155:1',
          kind: 'execution',
          execution: {
            type: 'evm_read',
            to: lit('0x1111111111111111111111111111111111111111'),
            abi: { type: 'function', name: 'q', inputs: [], outputs: [] },
            args: {},
          },
          writes: [{ path: 'nodes.q2.outputs', mode: 'set' }],
        },
      ],
    };

    const solver: Solver = { solve() { return {}; } };
    const executor: Executor = {
      supports() {
        return true;
      },
      async execute(node: ExecutionPlanNode) {
        return {
          outputs: { ok: true, id: node.id },
          patches: [{ op: 'set', path: `nodes.${node.id}.outputs`, value: { ok: true, id: node.id } }],
        };
      },
    };

    const trace = new MemoryTraceSink();
    const events: EngineEvent[] = [];
    for await (const ev of runPlan(plan, ctx, { solver, executors: [executor], trace: { sink: trace, run_id: 'test-run' } })) {
      events.push(ev);
    }

    expect(events.some((e) => e.type === 'plan_ready')).toBe(true);

    const root = trace.records.find((r) => r.kind === 'root');
    expect(root).toBeTruthy();
    expect(root?.run_id).toBe('test-run');

    const spans = trace.records.filter((r) => r.kind === 'node_span');
    expect(spans.map((s) => s.node_id).sort()).toEqual(['q1', 'q2']);
    expect(spans.every((s) => s.parent_id === root!.id)).toBe(true);

    const evs = trace.records.filter((r) => r.kind === 'event');
    expect(evs.length).toBeGreaterThan(0);
    expect(evs.every((r) => r.run_id === 'test-run')).toBe(true);

    const spanByNode = new Map(spans.map((s) => [s.node_id!, s.id]));
    const nodeEventParents = evs
      .filter((r) => typeof r.node_id === 'string')
      .map((r) => r.parent_id);
    expect(nodeEventParents).toContain(spanByNode.get('q1'));
    expect(nodeEventParents).toContain(spanByNode.get('q2'));
  });
});

