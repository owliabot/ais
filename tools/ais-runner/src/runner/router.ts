import { parseCliArgs, renderHelp } from '../cli.js';
import { loadRunnerConfig } from '../config.js';
import { loadSdk } from '../sdk.js';
import { planDiffCommand } from './commands/plan-diff.js';
import { runActionCommand } from './commands/run-action.js';
import { runPlanCommand } from './commands/run-plan.js';
import { runQueryCommand } from './commands/run-query.js';
import { runWorkflowCommand } from './commands/run-workflow.js';
import { replayCommand } from './commands/replay.js';

export async function run(argv: string[]): Promise<void> {
  const parsed = parseCliArgs(argv);
  if (parsed.kind === 'help') {
    process.stdout.write(renderHelp());
    return;
  }

  const sdk = await loadSdk();
  const config = (parsed as any).configPath ? await loadRunnerConfig((parsed as any).configPath) : null;

  if (parsed.kind === 'plan_diff') {
    await planDiffCommand({ parsed, sdk });
    return;
  }
  if (parsed.kind === 'replay') {
    await replayCommand({ parsed, sdk });
    return;
  }

  if (parsed.kind === 'run_workflow') {
    await runWorkflowCommand({ parsed, config, sdk });
    return;
  }
  if (parsed.kind === 'run_plan') {
    await runPlanCommand({ parsed, config, sdk });
    return;
  }
  if (parsed.kind === 'run_action') {
    await runActionCommand({ parsed, config, sdk });
    return;
  }
  await runQueryCommand({ parsed, config, sdk });
}
