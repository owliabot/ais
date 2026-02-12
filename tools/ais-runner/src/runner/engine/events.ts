import type { RunnerEngineEvent } from '../../types.js';

export function formatEvent(ev: RunnerEngineEvent): string {
  switch (ev.type) {
    case 'plan_ready':
      return 'event: plan_ready';
    case 'node_ready':
      return `event: node_ready node=${ev.node.id}`;
    case 'node_blocked': {
      const missing = ev.readiness.missing_refs?.join(',') ?? '';
      const detect = ev.readiness.needs_detect ? ' needs_detect' : '';
      return `event: node_blocked node=${ev.node.id} missing=[${missing}]${detect}`;
    }
    case 'solver_applied':
      return `event: solver_applied node=${ev.node.id} patches=${ev.patches.length}`;
    case 'query_result':
      return `event: query_result node=${ev.node.id}`;
    case 'tx_sent':
      return `event: tx_sent node=${ev.node.id} hash=${ev.tx_hash}`;
    case 'tx_confirmed':
      return `event: tx_confirmed node=${ev.node.id}`;
    case 'need_user_confirm':
      return `event: need_user_confirm node=${ev.node.id} reason=${String(ev.reason)}`;
    case 'node_waiting':
      return `event: node_waiting node=${ev.node.id} attempts=${ev.attempts}`;
    case 'skipped':
      return `event: skipped node=${ev.node.id}`;
    case 'engine_paused':
      return `event: engine_paused paused=${ev.paused.length}`;
    case 'command_accepted':
      return `event: command_accepted id=${ev.command.id} kind=${ev.command.kind}`;
    case 'command_rejected':
      return `event: command_rejected id=${ev.command?.id ?? 'unknown'} reason=${ev.reason}`;
    case 'patch_applied':
      return `event: patch_applied id=${ev.command?.id ?? 'unknown'} patches=${ev.summary.patch_count} hash=${ev.summary.hash.slice(0, 12)}`;
    case 'patch_rejected':
      return `event: patch_rejected id=${ev.command?.id ?? 'unknown'} reason=${ev.reason}`;
    case 'error':
      return `event: error node=${ev.node?.id ?? 'global'} msg=${ev.error?.message ?? String(ev.error)}`;
    case 'checkpoint_saved':
      return 'event: checkpoint_saved';
    case 'node_paused':
      return `event: node_paused node=${ev.node.id} reason=${ev.reason}`;
    case 'tx_prepared':
      return `event: tx_prepared node=${ev.node.id}`;
    default:
      return `event: ${String((ev as { type: string }).type)}`;
  }
}
