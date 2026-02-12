type CommonFlags = {
  configPath?: string;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
  traceRedactMode?: string;
  eventsJsonlPath?: string;
  commandsStdinJsonl?: boolean;
  outPath?: string;
  dryRun?: boolean;
  dryRunFormat?: string;
  broadcast?: boolean;
  yes?: boolean;
  strictImports?: boolean;
  importsOnly?: boolean;
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

export type RunPlanRequest = CommonFlags & {
  kind: 'run_plan';
  workspaceDir: string;
  filePath: string;
  workflowPath?: string;
  inputsJson?: string;
  ctxJson?: string;
};

export type PlanDiffRequest = {
  kind: 'plan_diff';
  aPath: string;
  bPath: string;
  format?: string; // text|json
};

export type ReplayRequest = {
  kind: 'replay';
  checkpointPath?: string;
  tracePath?: string;
  untilNodeId?: string;
  format?: string; // text|json
};

export type CliRequest =
  | RunWorkflowRequest
  | RunActionRequest
  | RunQueryRequest
  | RunPlanRequest
  | PlanDiffRequest
  | ReplayRequest
  | { kind: 'help' };

export function renderHelp(): string {
  return `AIS internal runner (ts-sdk verifier)

Usage:
  ais-runner run workflow --file <.ais-flow.yaml> --workspace <dir> [--inputs <json>] [--ctx <json>]
  ais-runner run plan --file <.ais-plan.json|.yaml> --workspace <dir> [--workflow <.ais-flow.yaml>] [--inputs <json>] [--ctx <json>]
  ais-runner run action --ref <protocol@ver>/<actionId> --workspace <dir> --args <json> [--chain <caip2>]
  ais-runner run query --ref <protocol@ver>/<queryId> --workspace <dir> --args <json> [--chain <caip2>]
  ais-runner plan diff --a <planA.json|yaml> --b <planB.json|yaml> [--format text|json]
  ais-runner replay --checkpoint <checkpoint.json> [--until-node <id>] [--format text|json]
  ais-runner replay --trace <trace.jsonl|events.jsonl> [--until-node <id>] [--format text|json]

Common options:
  --config <path>         Runner config (yaml)
  --checkpoint <path>     Checkpoint file path (json)
  --resume                Resume from checkpoint if compatible
  --trace <path>          Trace JSONL output path
  --trace-redact <mode>   Trace/Event redaction mode: default|audit|off (default: default)
  --events-jsonl <path|stdout>  Write raw engine events JSONL (use "stdout" or "-" for stdout)
  --commands-stdin-jsonl  Read command JSONL from stdin when engine pauses
  --out <path>            Write evaluated workflow outputs JSON
  --dry-run               Do not broadcast transactions
  --dry-run-format <fmt>  Dry-run output format: text|json (default: text)
  --broadcast             Allow broadcasting write transactions (default: false)
  --yes                   Auto-approve policy gates (default: false)
  --strict-imports        Enforce workflow imports allowlist (default: true)
  --imports-only          For workflow mode: load protocols from workflow.imports only
  -h, --help              Show help
`;
}

export function parseCliArgs(argv: string[]): CliRequest {
  const args = argv.slice();
  if (args.length === 0) return { kind: 'help' };
  if (args.includes('-h') || args.includes('--help')) return { kind: 'help' };

  const cmd = args[0];
  const mode = args[1];

  if (cmd === 'plan' && mode === 'diff') {
    const flags = parseFlags(args.slice(2));
    const aPath = flags.str['a'];
    const bPath = flags.str['b'];
    if (!aPath || !bPath) return { kind: 'help' };
    return { kind: 'plan_diff', aPath, bPath, format: flags.str['format'] };
  }

  if (cmd === 'replay') {
    const flags = parseFlags(args.slice(1));
    const checkpointPath = flags.str['checkpoint'];
    const tracePath = flags.str['trace'];
    if (!checkpointPath && !tracePath) return { kind: 'help' };
    return {
      kind: 'replay',
      checkpointPath,
      tracePath,
      untilNodeId: flags.str['until-node'],
      format: flags.str['format'],
    };
  }

  if (cmd !== 'run' || (mode !== 'workflow' && mode !== 'action' && mode !== 'query' && mode !== 'plan')) {
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

  if (mode === 'plan') {
    const filePath = flags.str['file'];
    const workspaceDir = flags.str['workspace'];
    if (!filePath || !workspaceDir) return { kind: 'help' };
    return {
      kind: 'run_plan',
      filePath,
      workflowPath: flags.str['workflow'],
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
    traceRedactMode: flags.str['trace-redact'],
    eventsJsonlPath: flags.str['events-jsonl'],
    commandsStdinJsonl: flags.bool.has('commands-stdin-jsonl'),
    outPath: flags.str['out'],
    dryRun: flags.bool.has('dry-run'),
    dryRunFormat: flags.str['dry-run-format'],
    broadcast: flags.bool.has('broadcast'),
    yes: flags.bool.has('yes'),
    strictImports: !flags.bool.has('no-strict-imports'),
    importsOnly: flags.bool.has('imports-only'),
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
    if (
      key === 'resume' ||
      key === 'dry-run' ||
      key === 'broadcast' ||
      key === 'yes' ||
      key === 'imports-only' ||
      key === 'commands-stdin-jsonl' ||
      key === 'no-strict-imports'
    ) {
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
