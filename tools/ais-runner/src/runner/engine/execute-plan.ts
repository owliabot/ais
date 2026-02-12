import { FileCheckpointStore } from '../../checkpoint-store.js';
import { formatPlanSummary } from '../../plan-print.js';
import { wrapSolverWithCalculatedFields } from '../../solver-wrappers.js';
import { createRunnerDetectResolver } from '../../detect.js';
import { evaluateWorkflowOutputs, stringifyWithBigInt, writeOutputsJson } from '../../output.js';
import { createInterface } from 'node:readline';
import type { RunnerConfig } from '../../config.js';
import type {
  RunnerContext,
  RunnerEngineCheckpoint,
  RunnerDestroyableExecutor,
  RunnerEngineEvent,
  RunnerPlan,
  RunnerRunPlanOptions,
  RunnerSdkModule,
  RunnerWorkflow,
  RunnerWorkspaceDocuments,
} from '../../types.js';
import { formatEvent } from './events.js';
import { destroyExecutors, missingSignerChains } from './execution-helpers.js';
import { applyRunnerSideEffects } from './side-effects.js';
import { findRequiredPackDocument } from '../workspace/resolve.js';
import { buildExecutors } from '../executors/build.js';
import { normalizeEventForJsonl } from './event-jsonl-map.js';
import { consumeCommandLines } from './commands-stdin.js';

type ExecuteFlags = {
  broadcast?: boolean;
  yes?: boolean;
  checkpointPath?: string;
  resume?: boolean;
  tracePath?: string;
  traceRedactMode?: string;
  eventsJsonlPath?: string;
  commandsStdinJsonl?: boolean;
};

type ExecutePlanArgs = {
  sdk: RunnerSdkModule;
  config: RunnerConfig | null;
  plan: RunnerPlan;
  context: RunnerContext;
  flags: ExecuteFlags;
  workflow?: RunnerWorkflow;
  workspaceDocs?: RunnerWorkspaceDocuments;
  writeOutputsPath?: string;
};

export async function executePlan(args: ExecutePlanArgs): Promise<void> {
  const { sdk, config, plan, context, flags, workflow, workspaceDocs, writeOutputsPath } = args;
  const broadcast = Boolean(flags.broadcast);
  const eventsTarget = resolveEventsJsonlTarget(flags.eventsJsonlPath);
  const humanOut = (eventsTarget.to_stdout ? (process as { stderr?: { write(chunk: string): void } }).stderr : process.stdout) ?? process.stdout;
  const redactMode = normalizeRedactMode(flags.traceRedactMode);
  if (!redactMode) {
    humanOut.write(`Invalid --trace-redact value: ${String(flags.traceRedactMode)} (expected default|audit|off)\n`);
    process.exitCode = 1;
    return;
  }

  if (!config) {
    humanOut.write('Missing --config for execution mode\n');
    process.exitCode = 1;
    return;
  }
  if (broadcast) {
    const missing = missingSignerChains(plan, config);
    if (missing.length > 0) {
      humanOut.write(`Missing signer config for broadcast on chains: ${missing.join(', ')}\n`);
      process.exitCode = 1;
      return;
    }
  }

  let executors: RunnerDestroyableExecutor[];
  try {
    executors = await buildExecutors({
      sdk,
      config,
      broadcast,
      yes: Boolean(flags.yes),
      pack: workflow && workspaceDocs ? findRequiredPackDocument(workflow, workspaceDocs)?.document : undefined,
    });
  } catch (error) {
    humanOut.write(`Failed to create executors from config: ${(error as Error)?.message ?? String(error)}\n`);
    process.exitCode = 1;
    return;
  }
  if (executors.length === 0) {
    humanOut.write('No executors created from config (missing/empty rpc_url?)\n');
    process.exitCode = 1;
    return;
  }

  const detect = createRunnerDetectResolver({ sdk, workflow, workspaceDocs });
  const baseSolver = sdk.createSolver ? sdk.createSolver() : sdk.solver;
  const solver = wrapSolverWithCalculatedFields({ sdk, inner: baseSolver, detect });
  const trace = flags.tracePath
    ? {
        sink: sdk.createJsonlTraceSink({ file_path: flags.tracePath }),
        redact_event: (ev: unknown) =>
          typeof sdk.redactEngineEventByMode === 'function'
            ? sdk.redactEngineEventByMode(ev as never, redactMode)
            : ev,
      }
    : undefined;
  const eventWriter = eventsTarget.file_path || eventsTarget.stream
    ? sdk.createEngineEventJsonlWriter({
        ...(eventsTarget.file_path ? { file_path: eventsTarget.file_path } : {}),
        ...(eventsTarget.stream ? { stream: eventsTarget.stream as never } : {}),
        map_event: (ev) =>
          mapEventForJsonl({
            sdk,
            ev,
            redact_mode: redactMode,
          }),
      })
    : null;
  const checkpoint_store = flags.checkpointPath
    ? new FileCheckpointStore(sdk, flags.checkpointPath, { redact_mode: redactMode })
    : new InMemoryCheckpointStore();
  const options: RunnerRunPlanOptions = {
    solver,
    executors,
    detect,
    max_concurrency: config.engine?.max_concurrency,
    per_chain: config.engine?.per_chain,
    trace,
    checkpoint_store,
    resume_from_checkpoint: Boolean(flags.resume || flags.commandsStdinJsonl),
  };

  humanOut.write(formatPlanSummary(plan));
  const seenCommandIds = await loadProcessedCommandIds(checkpoint_store);
  let endedEarly = false;
  try {
    while (true) {
      let pausedEvent: Extract<RunnerEngineEvent, { type: 'engine_paused' }> | null = null;
      let hasError = false;
      for await (const ev of sdk.runPlan(plan, context, options)) {
        appendEvent(eventWriter, ev);
        applyRunnerSideEffects(sdk, context, ev);
        humanOut.write(`${formatEvent(ev)}\n`);
        if (ev.type === 'engine_paused') pausedEvent = ev;
        if (ev.type === 'engine_paused' || ev.type === 'error') {
          hasError = ev.type === 'error';
          break;
        }
      }

      if (hasError) {
        endedEarly = true;
        break;
      }
      if (!pausedEvent) break;
      if (!flags.commandsStdinJsonl) {
        endedEarly = true;
        break;
      }

      const commandResult = await consumeCommandsFromStdin({
        sdk,
        plan,
        context,
        pausedNodeIds: new Set(pausedEvent.paused.map((entry) => entry.node.id)),
        eventWriter,
        seenCommandIds,
        humanOut,
      });
      await saveRunnerCommandStateToCheckpoint({
        checkpoint_store,
        sdk,
        context,
        seenCommandIds,
      });

      if (commandResult.cancel_requested || !commandResult.rerun_requested) {
        endedEarly = true;
        break;
      }
    }
  } finally {
    eventWriter?.close();
    await destroyExecutors(executors);
  }

  if (!endedEarly && workflow) {
    const evaluated = evaluateWorkflowOutputs(sdk, workflow, context);
    const payload = {
      kind: 'workflow_outputs',
      outputs: evaluated.outputs,
      errors: evaluated.errors,
    };
    process.stdout.write(`${stringifyWithBigInt(payload)}\n`);
    if (writeOutputsPath) await writeOutputsJson(writeOutputsPath, payload);
  }
}

export function resolveEventsJsonlTarget(path: string | undefined): {
  file_path?: string;
  stream?: unknown;
  to_stdout: boolean;
} {
  const raw = (path ?? '').trim();
  if (!raw) return { to_stdout: false };
  if (raw === '-' || raw.toLowerCase() === 'stdout') {
    return { stream: process.stdout as unknown, to_stdout: true };
  }
  return { file_path: raw, to_stdout: false };
}

function normalizeRedactMode(value: string | undefined): 'default' | 'audit' | 'off' | null {
  if (!value) return 'default';
  if (value === 'default' || value === 'audit' || value === 'off') return value;
  return null;
}

async function consumeCommandsFromStdin(args: {
  sdk: RunnerSdkModule;
  plan: RunnerPlan;
  context: RunnerContext;
  pausedNodeIds: Set<string>;
  eventWriter: ReturnType<RunnerSdkModule['createEngineEventJsonlWriter']> | null;
  seenCommandIds: Set<string>;
  humanOut: { write(chunk: string): void };
}): Promise<{ rerun_requested: boolean; cancel_requested: boolean }> {
  const { sdk, plan, context, pausedNodeIds, eventWriter, seenCommandIds, humanOut } = args;
  if (process.stdin.isTTY) {
    const ev: RunnerEngineEvent = {
      type: 'command_rejected',
      reason: '--commands-stdin-jsonl is enabled but stdin is a TTY (no JSONL input stream)',
      field_path: 'stdin',
      details: { expected: 'jsonl command stream' },
    };
    appendEvent(eventWriter, ev);
    humanOut.write(`${formatEvent(ev)}\n`);
    return { rerun_requested: false, cancel_requested: false };
  }

  const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
  const result = await consumeCommandLines({
    sdk,
    plan,
    context,
    lines: (async function* () {
      for await (const line of rl) yield String(line);
    })(),
    pausedNodeIds,
    seenCommandIds,
  });
  for (const ev of result.events) {
    appendEvent(eventWriter, ev);
    humanOut.write(`${formatEvent(ev)}\n`);
  }
  return {
    rerun_requested: result.rerun_requested,
    cancel_requested: result.cancel_requested,
  };
}

function mapEventForJsonl(args: {
  sdk: RunnerSdkModule;
  ev: RunnerEngineEvent;
  redact_mode: 'default' | 'audit' | 'off';
}): unknown {
  const { sdk, redact_mode } = args;
  const normalized = normalizeEventForJsonl(args.ev);
  const redacted =
    typeof sdk.redactEngineEventByMode === 'function'
      ? sdk.redactEngineEventByMode(normalized as never, redact_mode)
      : normalized;
  const envelope =
    typeof sdk.engineEventToEnvelope === 'function' ? sdk.engineEventToEnvelope(normalized as never) : null;

  const out = isEnvelopeLike(redacted)
    ? redacted
    : envelope
      ? { ...envelope, data: redacted }
      : { type: (args.ev as { type: string }).type, data: redacted };

  const existingExtensions = isRecord((out as { extensions?: unknown }).extensions)
    ? ((out as { extensions?: Record<string, unknown> }).extensions ?? {})
    : {};
  return {
    ...out,
    extensions: { ...existingExtensions, redaction_mode: redact_mode },
  };
}

function appendEvent(
  writer: ReturnType<RunnerSdkModule['createEngineEventJsonlWriter']> | null,
  ev: RunnerEngineEvent
): void {
  if (!writer) return;
  writer.append(ev as Parameters<typeof writer.append>[0]);
}

function isEnvelopeLike(value: unknown): value is { type: string; node_id?: string; data: unknown; extensions?: Record<string, unknown> } {
  if (!isRecord(value)) return false;
  if (typeof value.type !== 'string') return false;
  if (!('data' in value)) return false;
  if (value.node_id !== undefined && typeof value.node_id !== 'string') return false;
  if (value.extensions !== undefined && !isRecord(value.extensions)) return false;
  return true;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

class InMemoryCheckpointStore {
  private checkpoint: RunnerEngineCheckpoint | null = null;

  async load(): Promise<RunnerEngineCheckpoint | null> {
    return this.checkpoint ? deepClone(this.checkpoint) : null;
  }

  async save(checkpoint: RunnerEngineCheckpoint): Promise<void> {
    this.checkpoint = deepClone(checkpoint);
  }
}

async function loadProcessedCommandIds(
  checkpointStore: { load(): Promise<RunnerEngineCheckpoint | null> } | undefined
): Promise<Set<string>> {
  const out = new Set<string>();
  if (!checkpointStore) return out;
  const checkpoint = await checkpointStore.load();
  const ids = extractProcessedCommandIds(checkpoint);
  for (const id of ids) out.add(id);
  return out;
}

async function saveRunnerCommandStateToCheckpoint(args: {
  checkpoint_store: { load(): Promise<RunnerEngineCheckpoint | null>; save(checkpoint: RunnerEngineCheckpoint): Promise<void> } | undefined;
  sdk: RunnerSdkModule;
  context: RunnerContext;
  seenCommandIds: Set<string>;
}): Promise<void> {
  const { checkpoint_store, sdk, context, seenCommandIds } = args;
  if (!checkpoint_store) return;
  const checkpoint = await checkpoint_store.load();
  if (!checkpoint) return;
  const checkpointEx = checkpoint as RunnerEngineCheckpoint & { extensions?: Record<string, unknown> };

  checkpointEx.runtime = cloneBySdkCodec(sdk, context.runtime);
  const extensions = isRecord(checkpointEx.extensions) ? checkpointEx.extensions : {};
  const runnerState = isRecord(extensions.runner_command_state) ? extensions.runner_command_state : {};
  runnerState.processed_command_ids = Array.from(seenCommandIds);
  extensions.runner_command_state = runnerState;
  checkpointEx.extensions = extensions;
  await checkpoint_store.save(checkpointEx);
}

function extractProcessedCommandIds(checkpoint: RunnerEngineCheckpoint | null): string[] {
  const checkpointEx = checkpoint as (RunnerEngineCheckpoint & { extensions?: Record<string, unknown> }) | null;
  if (!checkpointEx || !isRecord(checkpointEx.extensions)) return [];
  const runnerState = isRecord(checkpointEx.extensions.runner_command_state) ? checkpointEx.extensions.runner_command_state : null;
  if (!runnerState || !Array.isArray(runnerState.processed_command_ids)) return [];
  return runnerState.processed_command_ids.filter((id: unknown): id is string => typeof id === 'string' && id.length > 0);
}

function deepClone<T>(value: T): T {
  const cloneFn = (globalThis as { structuredClone?: <U>(input: U) => U }).structuredClone;
  if (typeof cloneFn === 'function') return cloneFn(value);
  return JSON.parse(JSON.stringify(value)) as T;
}

function cloneBySdkCodec<T>(sdk: RunnerSdkModule, value: T): T {
  if (typeof sdk.stringifyAisJson === 'function' && typeof sdk.parseAisJson === 'function') {
    return sdk.parseAisJson(sdk.stringifyAisJson(value)) as T;
  }
  return deepClone(value);
}
