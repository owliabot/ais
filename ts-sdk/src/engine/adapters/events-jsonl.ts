import type { EngineEvent } from '../types.js';
import { createWriteStream } from 'node:fs';
import type { Writable } from 'node:stream';
import { stringifyAisJson, parseAisJson } from '../json.js';
import { randomUUID } from 'node:crypto';

export interface EngineEventEnvelope {
  type: EngineEvent['type'];
  node_id?: string;
  data: unknown;
  extensions?: Record<string, unknown>;
}

export interface EngineEventJsonlRecord {
  schema: 'ais-engine-event/0.0.3';
  run_id: string;
  seq: number;
  ts: string;
  event: EngineEventEnvelope;
}

export interface CreateEngineEventJsonlWriterOptions {
  /**
   * Append to a JSONL file. If provided, this takes precedence over `stream`.
   */
  file_path?: string;
  /**
   * Write JSONL records to an existing Writable stream.
   */
  stream?: Writable;
  /**
   * Stable run identifier. If omitted, a random UUID is generated.
   */
  run_id?: string;
  /**
   * Optional event mapper (e.g. redact or transform fields into envelope.data).
   *
   * If the mapper returns a full envelope shape (`{ type, data, ... }`), it is used directly.
   * Otherwise, the mapped value is assigned to `event.data` with the original type/node_id.
   */
  map_event?: (ev: EngineEvent) => unknown;
}

export interface EngineEventWriter {
  readonly run_id: string;
  append(ev: EngineEvent): void;
  close(): void;
}

export function createEngineEventJsonlWriter(options: CreateEngineEventJsonlWriterOptions): EngineEventWriter {
  const runId = options.run_id ?? randomUUID();
  const mapEvent = options.map_event;
  const stream =
    options.file_path ? createWriteStream(options.file_path, { flags: 'a' }) : options.stream;
  if (!stream) throw new Error('createEngineEventJsonlWriter requires file_path or stream');

  let seq = 0;
  return {
    run_id: runId,
    append(ev) {
      const mapped = mapEvent ? mapEvent(ev) : undefined;
      const envelope = toEventEnvelope(ev, mapped);
      const record: EngineEventJsonlRecord = {
        schema: 'ais-engine-event/0.0.3',
        run_id: runId,
        seq: seq++,
        ts: new Date().toISOString(),
        event: envelope,
      };
      stream.write(`${stringifyAisJson(record)}\n`);
    },
    close() {
      stream.end();
    },
  };
}

export function encodeEngineEventJsonlRecord(record: EngineEventJsonlRecord): string {
  return stringifyAisJson(record);
}

export function decodeEngineEventJsonlRecord(line: string): EngineEventJsonlRecord {
  const v = parseAisJson(line) as any;
  if (!v || typeof v !== 'object') throw new Error('Invalid JSONL record: not an object');
  if (v.schema !== 'ais-engine-event/0.0.3') throw new Error(`Invalid JSONL record schema: ${String(v.schema)}`);
  if (typeof v.run_id !== 'string') throw new Error('Invalid JSONL record: run_id must be string');
  if (typeof v.seq !== 'number' || !Number.isFinite(v.seq)) throw new Error('Invalid JSONL record: seq must be number');
  if (typeof v.ts !== 'string') throw new Error('Invalid JSONL record: ts must be string');
  if (!v.event || typeof v.event !== 'object') throw new Error('Invalid JSONL record: event must be object');
  if (typeof v.event.type !== 'string') throw new Error('Invalid JSONL record: event.type must be string');
  if (!('data' in v.event)) throw new Error('Invalid JSONL record: event.data is required');
  if (v.event.node_id !== undefined && typeof v.event.node_id !== 'string') {
    throw new Error('Invalid JSONL record: event.node_id must be string when provided');
  }
  if (v.event.extensions !== undefined && (typeof v.event.extensions !== 'object' || Array.isArray(v.event.extensions))) {
    throw new Error('Invalid JSONL record: event.extensions must be object when provided');
  }
  return v as EngineEventJsonlRecord;
}

export async function* engineEventsToJsonl(
  events: AsyncIterable<EngineEvent>,
  options: { run_id?: string; map_event?: (ev: EngineEvent) => unknown } = {}
): AsyncGenerator<string> {
  const runId = options.run_id ?? randomUUID();
  const mapEvent = options.map_event;
  let seq = 0;
  for await (const ev of events) {
    const mapped = mapEvent ? mapEvent(ev) : undefined;
    const envelope = toEventEnvelope(ev, mapped);
    const record: EngineEventJsonlRecord = {
      schema: 'ais-engine-event/0.0.3',
      run_id: runId,
      seq: seq++,
      ts: new Date().toISOString(),
      event: envelope,
    };
    yield `${encodeEngineEventJsonlRecord(record)}\n`;
  }
}

export function engineEventToEnvelope(ev: EngineEvent): EngineEventEnvelope {
  const nodeId = 'node' in ev && ev.node ? ev.node.id : undefined;
  switch (ev.type) {
    case 'plan_ready':
      return { type: ev.type, data: { plan: ev.plan } };
    case 'node_ready':
      return { type: ev.type, node_id: nodeId, data: {} };
    case 'node_blocked':
      return { type: ev.type, node_id: nodeId, data: { readiness: ev.readiness } };
    case 'node_paused':
      return { type: ev.type, node_id: nodeId, data: { reason: ev.reason, details: ev.details } };
    case 'solver_applied':
      return { type: ev.type, node_id: nodeId, data: { patches: ev.patches } };
    case 'query_result':
      return { type: ev.type, node_id: nodeId, data: { outputs: ev.outputs } };
    case 'tx_prepared':
      return { type: ev.type, node_id: nodeId, data: { tx: ev.tx } };
    case 'need_user_confirm':
      return { type: ev.type, node_id: nodeId, data: { reason: ev.reason, details: ev.details } };
    case 'tx_sent':
      return { type: ev.type, node_id: nodeId, data: { tx_hash: ev.tx_hash } };
    case 'tx_confirmed':
      return { type: ev.type, node_id: nodeId, data: { receipt: ev.receipt } };
    case 'node_waiting':
      return {
        type: ev.type,
        node_id: nodeId,
        data: { attempts: ev.attempts, next_attempt_at_ms: ev.next_attempt_at_ms },
      };
    case 'engine_paused':
      return { type: ev.type, data: { paused: ev.paused } };
    case 'skipped':
      return { type: ev.type, node_id: nodeId, data: { reason: ev.reason } };
    case 'command_accepted':
      return {
        type: ev.type,
        data: {
          command: ev.command,
          details: ev.details,
        },
      };
    case 'command_rejected':
      return {
        type: ev.type,
        data: {
          command: ev.command,
          reason: ev.reason,
          field_path: ev.field_path,
          details: ev.details,
        },
      };
    case 'patch_applied':
      return {
        type: ev.type,
        data: {
          command: ev.command,
          summary: ev.summary,
          details: ev.details,
        },
      };
    case 'patch_rejected':
      return {
        type: ev.type,
        data: {
          command: ev.command,
          reason: ev.reason,
          field_path: ev.field_path,
          summary: ev.summary,
          details: ev.details,
        },
      };
    case 'error':
      return {
        type: ev.type,
        node_id: nodeId,
        data: {
          reason: ev.error?.message ?? String(ev.error),
          retryable: Boolean(ev.retryable),
          error: ev.error,
        },
      };
    case 'checkpoint_saved':
      return { type: ev.type, data: { checkpoint: ev.checkpoint } };
    default: {
      const _exhaustive: never = ev;
      return _exhaustive;
    }
  }
}

function toEventEnvelope(ev: EngineEvent, mapped: unknown): EngineEventEnvelope {
  if (isEnvelopeLike(mapped)) {
    return {
      type: mapped.type,
      node_id: mapped.node_id,
      data: mapped.data,
      extensions: mapped.extensions,
    };
  }
  const base = engineEventToEnvelope(ev);
  if (mapped === undefined) return base;
  return { ...base, data: mapped };
}

function isEnvelopeLike(value: unknown): value is EngineEventEnvelope {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const rec = value as Record<string, unknown>;
  if (typeof rec.type !== 'string') return false;
  if (!('data' in rec)) return false;
  if (rec.node_id !== undefined && typeof rec.node_id !== 'string') return false;
  if (rec.extensions !== undefined && (typeof rec.extensions !== 'object' || Array.isArray(rec.extensions))) return false;
  return true;
}
