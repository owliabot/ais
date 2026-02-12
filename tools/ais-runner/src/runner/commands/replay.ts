import { readFile } from 'node:fs/promises';
import type { ReplayRequest } from '../../cli.js';
import type { RunnerEngineEvent, RunnerSdkModule } from '../../types.js';
import { formatEvent } from '../engine/events.js';

export async function replayCommand(args: { parsed: ReplayRequest; sdk: RunnerSdkModule }): Promise<void> {
  const { parsed, sdk } = args;
  const fmt = String(parsed.format ?? 'text');
  const until = parsed.untilNodeId ? String(parsed.untilNodeId) : '';

  if (parsed.checkpointPath) {
    const raw = await readFile(parsed.checkpointPath, 'utf-8');
    const checkpoint = sdk.deserializeCheckpoint(raw) as any;
    const events: RunnerEngineEvent[] = Array.isArray(checkpoint?.events) ? checkpoint.events : [];
    const sliced = until ? sliceUntilNode(events, until) : events;
    if (fmt === 'json') {
      process.stdout.write(
        `${JSON.stringify(
          {
            kind: 'replay',
            source: 'checkpoint',
            path: parsed.checkpointPath,
            until_node: until || undefined,
            checkpoint_summary: summarizeCheckpoint(checkpoint),
            events: sliced,
          },
          null,
          2
        )}\n`
      );
      return;
    }
    const lines: string[] = [];
    lines.push('== replay ==');
    lines.push(`source=checkpoint path=${parsed.checkpointPath}`);
    if (until) lines.push(`until_node=${until}`);
    if (!Array.isArray(checkpoint?.events)) {
      const summary = summarizeCheckpoint(checkpoint);
      lines.push('note=checkpoint has no embedded events (runPlan option include_events_in_checkpoint=false)');
      lines.push(`checkpoint.created_at=${String(summary.created_at ?? '')}`);
      lines.push(`checkpoint.completed_nodes=${String(summary.completed_node_ids ?? []).length}`);
      lines.push(`checkpoint.paused_nodes=${String(summary.paused_node_ids ?? []).length}`);
      process.stdout.write(`${lines.join('\n')}\n`);
      return;
    }
    for (const ev of sliced) lines.push(formatEvent(ev));
    process.stdout.write(`${lines.join('\n')}\n`);
    return;
  }

  if (parsed.tracePath) {
    const raw = await readFile(parsed.tracePath, 'utf-8');
    const linesIn = raw.split(/\r?\n/).filter((l) => l.trim().length > 0);
    const outEvents: RunnerEngineEvent[] = [];
    for (const line of linesIn) {
      let obj: any;
      try {
        obj = sdk.parseAisJson(line);
      } catch {
        continue;
      }
      const ev = extractEventFromJsonl(obj);
      if (!ev) continue;
      outEvents.push(ev);
      if (until && eventHasNode(ev, until)) break;
    }
    if (fmt === 'json') {
      process.stdout.write(
        `${JSON.stringify({ kind: 'replay', source: 'trace', path: parsed.tracePath, until_node: until || undefined, events: outEvents }, null, 2)}\n`
      );
      return;
    }
    const lines: string[] = [];
    lines.push('== replay ==');
    lines.push(`source=trace path=${parsed.tracePath}`);
    if (until) lines.push(`until_node=${until}`);
    for (const ev of outEvents) lines.push(formatEvent(ev));
    process.stdout.write(`${lines.join('\n')}\n`);
    return;
  }

  process.stdout.write(`${JSON.stringify({ kind: 'replay_error', message: 'missing --checkpoint or --trace' }, null, 2)}\n`);
  process.exitCode = 1;
}

function sliceUntilNode(events: RunnerEngineEvent[], untilNodeId: string): RunnerEngineEvent[] {
  const out: RunnerEngineEvent[] = [];
  for (const ev of events) {
    out.push(ev);
    if (eventHasNode(ev, untilNodeId)) break;
  }
  return out;
}

function eventHasNode(ev: RunnerEngineEvent, nodeId: string): boolean {
  const nid = (ev as any)?.node?.id;
  if (typeof nid === 'string' && nid === nodeId) return true;
  if ((ev as any)?.type === 'need_user_confirm' && typeof (ev as any)?.node?.id === 'string' && (ev as any).node.id === nodeId) return true;
  return false;
}

function extractEventFromJsonl(obj: any): RunnerEngineEvent | null {
  // Engine trace record
  if (obj && typeof obj === 'object' && obj.kind === 'event' && obj.data && typeof obj.data === 'object' && typeof obj.data.type === 'string') {
    return obj.data as RunnerEngineEvent;
  }
  // Engine event JSONL record
  if (obj && typeof obj === 'object' && obj.schema === 'ais-engine-event/0.0.3' && obj.event && typeof obj.event === 'object') {
    const env = obj.event as any;
    if (env?.data && typeof env.data === 'object' && typeof env.type === 'string') {
      // If the record stored raw event (not envelope), fall back.
      const maybeEv = env.data;
      if (maybeEv && typeof maybeEv.type === 'string') return maybeEv as RunnerEngineEvent;
      // Otherwise attempt reconstruct minimal EngineEvent from envelope.
      if (env.type === 'need_user_confirm') {
        const data = env.data as any;
        if (data?.reason) {
          return { type: 'need_user_confirm', node: { id: env.node_id, chain: '', kind: 'execution', execution: { type: '' } } as any, reason: data.reason, details: data.details } as any;
        }
      }
    }
  }
  return null;
}

function summarizeCheckpoint(checkpoint: any): {
  schema?: string;
  created_at?: string;
  plan_schema?: string;
  plan_node_count?: number;
  completed_node_ids?: string[];
  paused_node_ids?: string[];
} {
  const completed = Array.isArray(checkpoint?.completed_node_ids)
    ? checkpoint.completed_node_ids.filter((x: any) => typeof x === 'string')
    : [];
  const paused = checkpoint?.paused_by_node_id && typeof checkpoint.paused_by_node_id === 'object'
    ? Object.keys(checkpoint.paused_by_node_id).filter((x) => typeof x === 'string')
    : [];
  const nodes = Array.isArray(checkpoint?.plan?.nodes) ? checkpoint.plan.nodes : [];
  return {
    schema: typeof checkpoint?.schema === 'string' ? checkpoint.schema : undefined,
    created_at: typeof checkpoint?.created_at === 'string' ? checkpoint.created_at : undefined,
    plan_schema: typeof checkpoint?.plan?.schema === 'string' ? checkpoint.plan.schema : undefined,
    plan_node_count: nodes.length,
    completed_node_ids: completed,
    paused_node_ids: paused,
  };
}
