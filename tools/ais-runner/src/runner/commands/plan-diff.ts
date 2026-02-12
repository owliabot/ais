import type { PlanDiffRequest } from '../../cli.js';
import type { RunnerSdkModule } from '../../types.js';
import { loadAndValidatePlanFile } from './run-plan.js';
import { diffPlans } from '../../plan-diff.js';

export async function planDiffCommand(args: { parsed: PlanDiffRequest; sdk: RunnerSdkModule }): Promise<void> {
  const { parsed, sdk } = args;
  const a = await loadAndValidatePlanFile(sdk, parsed.aPath);
  if (!a.ok) {
    process.stdout.write(`${JSON.stringify({ kind: 'plan_diff_error', side: 'a', error: a.error }, null, 2)}\n`);
    process.exitCode = 1;
    return;
  }
  const b = await loadAndValidatePlanFile(sdk, parsed.bPath);
  if (!b.ok) {
    process.stdout.write(`${JSON.stringify({ kind: 'plan_diff_error', side: 'b', error: b.error }, null, 2)}\n`);
    process.exitCode = 1;
    return;
  }

  const result = diffPlans(a.plan, b.plan);
  const fmt = String(parsed.format ?? 'text');
  if (fmt === 'json') {
    process.stdout.write(`${JSON.stringify({ ...result, a: { path: parsed.aPath }, b: { path: parsed.bPath } }, null, 2)}\n`);
    return;
  }

  const lines: string[] = [];
  lines.push('== plan diff ==');
  lines.push(`a=${parsed.aPath}`);
  lines.push(`b=${parsed.bPath}`);
  lines.push(`added=${result.summary.added} removed=${result.summary.removed} changed=${result.summary.changed}`);
  if (result.added.length > 0) lines.push(`added_nodes=${result.added.join(',')}`);
  if (result.removed.length > 0) lines.push(`removed_nodes=${result.removed.join(',')}`);
  for (const ch of result.changed) {
    lines.push(`# node ${ch.id}`);
    for (const c of ch.changes) {
      lines.push(`- ${c.field}`);
    }
  }
  process.stdout.write(`${lines.join('\n')}\n`);
}

