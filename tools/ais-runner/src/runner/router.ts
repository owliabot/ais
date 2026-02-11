import { parseCliArgs, renderHelp } from '../cli.js';
import { loadRunnerConfig } from '../config.js';
import { loadSdk } from '../sdk.js';
import { runActionCommand } from './commands/run-action.js';
import { runQueryCommand } from './commands/run-query.js';
import { runWorkflowCommand } from './commands/run-workflow.js';

export async function run(argv: string[]): Promise<void> {
  const parsed = parseCliArgs(argv);
  if (parsed.kind === 'help') {
    process.stdout.write(renderHelp());
    return;
  }

  const config = parsed.configPath ? await loadRunnerConfig(parsed.configPath) : null;
  const sdk = await loadSdk();

  if (parsed.kind === 'run_workflow') {
    await runWorkflowCommand({ parsed, config, sdk });
    return;
  }
  if (parsed.kind === 'run_action') {
    await runActionCommand({ parsed, config, sdk });
    return;
  }
  await runQueryCommand({ parsed, config, sdk });
}
