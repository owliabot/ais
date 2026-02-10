type CommonFlags = {
  configPath?: string;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
  outPath?: string;
  dryRun?: boolean;
  broadcast?: boolean;
  yes?: boolean;
};

export type RunWorkflowRequest = CommonFlags & {
  kind: 'run_workflow';
  workspaceDir: string;
  filePath: string;
  inputsJson?: string;
  ctxJson?: string;
};

export type RunActionRequest = CommonFlags & {
  kind: 'run_action';
  workspaceDir: string;
  actionRef: string; // protocol@ver/actionId
  argsJson: string;
  chain?: string;
};

export type RunQueryRequest = CommonFlags & {
  kind: 'run_query';
  workspaceDir: string;
  queryRef: string; // protocol@ver/queryId
  argsJson: string;
  chain?: string;
  untilCel?: string;
  retryJson?: string;
  timeoutMs?: number;
};

export type CliRequest = RunWorkflowRequest | RunActionRequest | RunQueryRequest | { kind: 'help' };

export function renderHelp(): string {
  return `AIS internal runner (ts-sdk verifier)

Usage:
  ais-runner run workflow --file <.ais-flow.yaml> --workspace <dir> [--inputs <json>] [--ctx <json>]
  ais-runner run action --ref <protocol@ver>/<actionId> --workspace <dir> --args <json> [--chain <caip2>]
  ais-runner run query --ref <protocol@ver>/<queryId> --workspace <dir> --args <json> [--chain <caip2>]

Common options:
  --config <path>         Runner config (yaml)
  --checkpoint <path>     Checkpoint file path (json)
  --resume                Resume from checkpoint if compatible
  --trace <path>          Trace JSONL output path
  --out <path>            Write evaluated workflow outputs JSON
  --dry-run               Do not broadcast transactions
  --broadcast             Allow broadcasting write transactions (default: false)
  --yes                   Auto-approve policy gates (default: false)
  -h, --help              Show help
`;
}

export function parseCliArgs(argv: string[]): CliRequest {
  const args = argv.slice();
  if (args.length === 0) return { kind: 'help' };
  if (args.includes('-h') || args.includes('--help')) return { kind: 'help' };

  const cmd = args[0];
  const mode = args[1];
  if (cmd !== 'run' || (mode !== 'workflow' && mode !== 'action' && mode !== 'query')) {
    return { kind: 'help' };
  }

  const flags = parseFlags(args.slice(2));
  if (mode === 'workflow') {
    const filePath = flags.str['file'];
    const workspaceDir = flags.str['workspace'];
    if (!filePath || !workspaceDir) return { kind: 'help' };
    return {
      kind: 'run_workflow',
      filePath,
      workspaceDir,
      inputsJson: flags.str['inputs'],
      ctxJson: flags.str['ctx'],
      ...commonFromFlags(flags),
    };
  }

  if (mode === 'action') {
    const actionRef = flags.str['ref'];
    const workspaceDir = flags.str['workspace'];
    const argsJson = flags.str['args'];
    if (!actionRef || !workspaceDir || !argsJson) return { kind: 'help' };
    return {
      kind: 'run_action',
      actionRef,
      workspaceDir,
      argsJson,
      chain: flags.str['chain'],
      ...commonFromFlags(flags),
    };
  }

  const queryRef = flags.str['ref'];
  const workspaceDir = flags.str['workspace'];
  const argsJson = flags.str['args'];
  if (!queryRef || !workspaceDir || !argsJson) return { kind: 'help' };
  return {
    kind: 'run_query',
    queryRef,
    workspaceDir,
    argsJson,
    chain: flags.str['chain'],
    untilCel: flags.str['until'],
    retryJson: flags.str['retry'],
    timeoutMs: flags.num['timeout-ms'],
    ...commonFromFlags(flags),
  };
}

function commonFromFlags(flags: ParsedFlags): CommonFlags {
  return {
    configPath: flags.str['config'],
    checkpointPath: flags.str['checkpoint'],
    resume: flags.bool.has('resume'),
    tracePath: flags.str['trace'],
    outPath: flags.str['out'],
    dryRun: flags.bool.has('dry-run'),
    broadcast: flags.bool.has('broadcast'),
    yes: flags.bool.has('yes'),
  };
}

type ParsedFlags = {
  str: Record<string, string | undefined>;
  num: Record<string, number | undefined>;
  bool: Set<string>;
};

function parseFlags(argv: string[]): ParsedFlags {
  const str: ParsedFlags['str'] = {};
  const num: ParsedFlags['num'] = {};
  const bool = new Set<string>();

  const args = argv.slice();
  for (let i = 0; i < args.length; i++) {
    const a = args[i]!;
    if (!a.startsWith('--')) continue;
    const key = a.slice(2);
    if (key === 'resume' || key === 'dry-run' || key === 'broadcast' || key === 'yes') {
      bool.add(key);
      continue;
    }
    const next = args[i + 1];
    if (!next || next.startsWith('-')) continue;
    i++;
    if (key === 'timeout-ms') {
      const n = Number(next);
      num[key] = Number.isFinite(n) ? n : undefined;
    } else {
      str[key] = next;
    }
  }

  return { str, num, bool };
}
