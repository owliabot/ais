import type { ExecutionPlan, ExecutionPlanNode } from '../execution/index.js';
import type { EngineEvent } from './types.js';
import { createWriteStream } from 'node:fs';
import type { Writable } from 'node:stream';

export type ExecutionTraceRecordKind = 'root' | 'node_span' | 'event';

export interface ExecutionTraceRecord {
  kind: ExecutionTraceRecordKind;
  id: string;
  parent_id?: string;
  run_id: string;
  seq: number;
  ts: string;
  node_id?: string;
  data: unknown;
}

export interface ExecutionTraceSink {
  append(record: ExecutionTraceRecord): void | Promise<void>;
  close?(): void | Promise<void>;
}

export interface CreateJsonlTraceSinkOptions {
  file_path: string;
}

export function createJsonlTraceSink(options: CreateJsonlTraceSinkOptions): ExecutionTraceSink {
  const stream = createWriteStream(options.file_path, { flags: 'a' });
  return createJsonlTraceSinkFromWritable(stream);
}

export function createJsonlTraceSinkFromWritable(stream: Writable): ExecutionTraceSink {
  return {
    append(record) {
      stream.write(`${JSON.stringify(record)}\n`);
    },
    close() {
      stream.end();
    },
  };
}

export function redactPlanForTrace(plan: ExecutionPlan): unknown {
  return {
    schema: plan.schema,
    meta: plan.meta ?? undefined,
    nodes: plan.nodes.map((n) => ({
      id: n.id,
      chain: n.chain,
      kind: n.kind,
      exec_type: n.execution.type,
      deps: n.deps ?? [],
    })),
  };
}

export function redactNodeForTrace(node: ExecutionPlanNode): unknown {
  return {
    id: node.id,
    chain: node.chain,
    kind: node.kind,
    exec_type: node.execution.type,
    deps: node.deps ?? [],
    source: node.source ?? undefined,
  };
}

export function redactEngineEventForTrace(ev: EngineEvent): unknown {
  switch (ev.type) {
    case 'plan_ready':
      return { type: ev.type, plan: redactPlanForTrace(ev.plan) };
    case 'node_ready':
    case 'skipped':
    case 'need_user_confirm':
    case 'node_paused':
    case 'node_waiting':
    case 'tx_prepared':
    case 'tx_sent':
    case 'tx_confirmed':
    case 'query_result':
    case 'node_blocked':
    case 'solver_applied':
      return {
        ...('node' in ev ? { node: redactNodeForTrace(ev.node) } : {}),
        ...redactEventFields(ev),
      };
    case 'engine_paused':
      return {
        type: ev.type,
        paused: ev.paused.map((p) => ({
          node: redactNodeForTrace(p.node),
          reason: p.reason,
          details: p.details,
        })),
      };
    case 'error':
      return {
        type: ev.type,
        node: ev.node ? redactNodeForTrace(ev.node) : undefined,
        error: { name: ev.error.name, message: ev.error.message, stack: ev.error.stack },
        retryable: ev.retryable ?? undefined,
      };
    case 'checkpoint_saved':
      return {
        type: ev.type,
        checkpoint: {
          schema: ev.checkpoint.schema,
          created_at: ev.checkpoint.created_at,
          completed_node_ids: ev.checkpoint.completed_node_ids,
          poll_state_by_node_id: ev.checkpoint.poll_state_by_node_id,
          paused_by_node_id: ev.checkpoint.paused_by_node_id,
        },
      };
    default: {
      const _exhaustive: never = ev;
      return _exhaustive;
    }
  }
}

function redactEventFields(ev: EngineEvent): Record<string, unknown> {
  switch (ev.type) {
    case 'node_ready':
      return { type: ev.type };
    case 'node_blocked':
      return { type: ev.type, readiness: ev.readiness };
    case 'solver_applied':
      return { type: ev.type, patches: ev.patches };
    case 'query_result':
      return { type: ev.type, outputs: ev.outputs };
    case 'need_user_confirm':
      return { type: ev.type, reason: ev.reason, details: ev.details };
    case 'node_paused':
      return { type: ev.type, reason: ev.reason, details: ev.details };
    case 'node_waiting':
      return { type: ev.type, attempts: ev.attempts, next_attempt_at_ms: ev.next_attempt_at_ms };
    case 'tx_prepared':
      return { type: ev.type, tx: ev.tx };
    case 'tx_sent':
      return { type: ev.type, tx_hash: ev.tx_hash };
    case 'tx_confirmed':
      return { type: ev.type, receipt: ev.receipt };
    case 'skipped':
      return { type: ev.type, reason: ev.reason };
    default:
      return { type: (ev as EngineEvent).type };
  }
}
