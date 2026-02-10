import type { EngineEvent } from '../types.js';
import { createWriteStream } from 'node:fs';
import type { Writable } from 'node:stream';
import { stringifyAisJson, parseAisJson } from '../json.js';
import { randomUUID } from 'node:crypto';

export interface EngineEventJsonlRecord {
  schema: 'ais-engine-event/0.0.2';
  run_id: string;
  seq: number;
  ts: string;
  event: unknown;
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
   * Optional event mapper (e.g. redact or transform fields).
   *
   * Default is identity (writes full EngineEvent).
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
  const mapEvent = options.map_event ?? ((ev: EngineEvent) => ev);
  const stream =
    options.file_path ? createWriteStream(options.file_path, { flags: 'a' }) : options.stream;
  if (!stream) throw new Error('createEngineEventJsonlWriter requires file_path or stream');

  let seq = 0;
  return {
    run_id: runId,
    append(ev) {
      const record: EngineEventJsonlRecord = {
        schema: 'ais-engine-event/0.0.2',
        run_id: runId,
        seq: seq++,
        ts: new Date().toISOString(),
        event: mapEvent(ev),
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
  if (v.schema !== 'ais-engine-event/0.0.2') throw new Error(`Invalid JSONL record schema: ${String(v.schema)}`);
  if (typeof v.run_id !== 'string') throw new Error('Invalid JSONL record: run_id must be string');
  if (typeof v.seq !== 'number' || !Number.isFinite(v.seq)) throw new Error('Invalid JSONL record: seq must be number');
  if (typeof v.ts !== 'string') throw new Error('Invalid JSONL record: ts must be string');
  return v as EngineEventJsonlRecord;
}

export async function* engineEventsToJsonl(
  events: AsyncIterable<EngineEvent>,
  options: { run_id?: string; map_event?: (ev: EngineEvent) => unknown } = {}
): AsyncGenerator<string> {
  const runId = options.run_id ?? randomUUID();
  const mapEvent = options.map_event ?? ((ev: EngineEvent) => ev);
  let seq = 0;
  for await (const ev of events) {
    const record: EngineEventJsonlRecord = {
      schema: 'ais-engine-event/0.0.2',
      run_id: runId,
      seq: seq++,
      ts: new Date().toISOString(),
      event: mapEvent(ev),
    };
    yield `${encodeEngineEventJsonlRecord(record)}\n`;
  }
}

