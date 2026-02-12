import type { ExecutionPlan, ExecutionPlanNode } from '../execution/index.js';
import type { EngineEvent } from './types.js';
import { createWriteStream } from 'node:fs';
import type { Writable } from 'node:stream';
import { stringifyAisJson } from './json.js';

export type TraceRedactMode = 'default' | 'audit' | 'off';

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
      stream.write(`${stringifyAisJson(record)}\n`);
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
    case 'command_accepted':
    case 'command_rejected':
    case 'patch_applied':
    case 'patch_rejected':
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

export function redactEngineEventByMode(
  ev: EngineEvent,
  mode: TraceRedactMode = 'default',
  options: { allow_path_patterns?: string[] } = {}
): unknown {
  if (mode === 'off') return ev;
  if (mode === 'audit') {
    return redactSensitiveFields(ev, { strict: false, allow_path_patterns: options.allow_path_patterns });
  }
  return redactSensitiveFields(redactEngineEventForTrace(ev), { strict: true, allow_path_patterns: options.allow_path_patterns });
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
    case 'command_accepted':
      return { type: ev.type, command: ev.command, details: ev.details };
    case 'command_rejected':
      return {
        type: ev.type,
        command: ev.command,
        reason: ev.reason,
        field_path: ev.field_path,
        details: ev.details,
      };
    case 'patch_applied':
      return {
        type: ev.type,
        command: ev.command,
        summary: ev.summary,
        details: ev.details,
      };
    case 'patch_rejected':
      return {
        type: ev.type,
        command: ev.command,
        reason: ev.reason,
        field_path: ev.field_path,
        summary: ev.summary,
        details: ev.details,
      };
    default:
      return { type: (ev as EngineEvent).type };
  }
}

const SECRET_KEY_PATTERNS: RegExp[] = [
  /private.?key/i,
  /seed/i,
  /mnemonic/i,
  /passphrase/i,
  /secret/i,
  /signature.?material/i,
  /signed.?tx/i,
  /^auth(orization)?$/i,
  // Keep "token" redaction narrowly scoped to auth contexts (do not redact token_address/token_symbol etc).
  /api[_-]?token/i,
  /access[_-]?token/i,
  /auth[_-]?token/i,
  /refresh[_-]?token/i,
  /pii/i,
];

const STRICT_ONLY_PATTERNS: RegExp[] = [
  /rpc.?payload/i,
  /rpc.?request/i,
  /rpc.?response/i,
  /raw.?tx/i,
  /raw.?transaction/i,
  /^tx$/i,
];

export function redactSensitiveFields(
  value: unknown,
  options: { strict: boolean; allow_path_patterns?: string[] }
): unknown {
  return redactSensitiveFieldsAtPath(value, options, []);
}

function redactSensitiveFieldsAtPath(
  value: unknown,
  options: { strict: boolean; allow_path_patterns?: string[] },
  path: Array<string | number>
): unknown {
  if (Array.isArray(value)) {
    return value.map((entry, i) => redactSensitiveFieldsAtPath(entry, options, [...path, i]));
  }
  if (!value || typeof value !== 'object') return value;
  // Preserve non-plain objects (Uint8Array, Buffer, class instances, etc) to avoid
  // breaking codec roundtrips in checkpoints/traces.
  if (!isPlainObject(value)) return value;

  const input = value as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const [key, entry] of Object.entries(input)) {
    const fullPath = [...path, key];
    if (shouldRedactKey(key, fullPath, options.strict, options.allow_path_patterns)) {
      out[key] = '[REDACTED]';
      continue;
    }
    out[key] = redactSensitiveFieldsAtPath(entry, options, fullPath);
  }
  return out;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== 'object') return false;
  if (Array.isArray(value)) return false;
  if (typeof Buffer !== 'undefined' && Buffer.isBuffer(value)) return false;
  if (ArrayBuffer.isView(value)) return false; // includes Uint8Array, etc
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

function shouldRedactKey(
  key: string,
  fullPath: Array<string | number>,
  strict: boolean,
  allowPathPatterns: string[] | undefined
): boolean {
  const pathStr = fieldPath(fullPath);
  if (Array.isArray(allowPathPatterns) && allowPathPatterns.length > 0) {
    for (const p of allowPathPatterns) {
      try {
        if (new RegExp(p).test(pathStr)) return false;
      } catch {
        // ignore invalid regex patterns
      }
    }
  }
  if (SECRET_KEY_PATTERNS.some((pattern) => pattern.test(key))) return true;
  if (strict && STRICT_ONLY_PATTERNS.some((pattern) => pattern.test(key))) return true;
  return false;
}

function fieldPath(path: Array<string | number>): string {
  if (path.length === 0) return '(root)';
  let out = '';
  for (const seg of path) {
    if (typeof seg === 'number') {
      out += `[${seg}]`;
      continue;
    }
    if (!out) out = seg;
    else out += `.${seg}`;
  }
  return out;
}
