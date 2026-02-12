import type { ExecutionPlan, ExecutionPlanNode } from '../execution/index.js';
import { getNodeReadiness, getNodeReadinessAsync } from '../execution/index.js';
import type { ResolverContext } from '../resolver/index.js';
import { evaluateValueRef, evaluateValueRefAsync, ValueRefEvalError } from '../resolver/index.js';
import { applyRuntimePatches } from './patch.js';
import { randomUUID } from 'node:crypto';
import type {
  CheckpointStore,
  EngineCheckpoint,
  EngineEvent,
  Executor,
  Solver,
  NodePauseState,
  NodePollState,
} from './types.js';
import type { DetectResolver } from '../resolver/value-ref.js';
import type { ExecutionTraceSink } from './trace.js';
import { redactEngineEventForTrace, redactNodeForTrace, redactPlanForTrace } from './trace.js';
import { summarizeNeedUserConfirm } from './confirm-summary.js';

export interface RunPlanOptions {
  solver: Solver;
  executors: Executor[];
  detect?: DetectResolver;
  checkpoint_store?: CheckpointStore;
  resume_from_checkpoint?: boolean;
  include_events_in_checkpoint?: boolean;
  stop_on_error?: boolean;
  max_concurrency?: number;
  per_chain?: Record<
    string,
    {
      max_read_concurrency?: number;
      max_write_concurrency?: number;
    }
  >;
  trace?: {
    sink: ExecutionTraceSink;
    run_id?: string;
    redact_event?: (ev: EngineEvent) => unknown;
  };
}

type NodeIoKind = 'read' | 'write';

export async function* runPlan(
  plan: ExecutionPlan,
  ctx: ResolverContext,
  options: RunPlanOptions
): AsyncGenerator<EngineEvent> {
  const stopOnError = options.stop_on_error ?? true;
  const maxConcurrency = options.max_concurrency ?? 8;
  const resume = options.resume_from_checkpoint ?? true;

  const completed = new Set<string>();
  const pausedByNodeId = new Map<string, NodePauseState>();
  const pollStateByNodeId = new Map<string, NodePollState>();
  const eventsForCheckpoint: EngineEvent[] = [];

  const traceSink = options.trace?.sink;
  const traceRunId = traceSink ? (options.trace?.run_id ?? randomUUID()) : null;
  const traceRootId = traceSink ? randomUUID() : null;
  const traceSeq = { n: 0 };
  const nodeSpanIdByNodeId = new Map<string, string>();
  const redactEvent = options.trace?.redact_event ?? redactEngineEventForTrace;

  const traceAppend = async (record: {
    kind: 'root' | 'node_span' | 'event';
    id: string;
    parent_id?: string;
    node_id?: string;
    data: unknown;
  }) => {
    if (!traceSink || !traceRunId) return;
    await traceSink.append({
      kind: record.kind,
      id: record.id,
      parent_id: record.parent_id,
      run_id: traceRunId,
      seq: traceSeq.n++,
      ts: new Date().toISOString(),
      node_id: record.node_id,
      data: record.data,
    });
  };

  const ensureNodeSpan = async (node: ExecutionPlanNode): Promise<string> => {
    const existing = nodeSpanIdByNodeId.get(node.id);
    if (existing) return existing;
    const id = randomUUID();
    nodeSpanIdByNodeId.set(node.id, id);
    await traceAppend({
      kind: 'node_span',
      id,
      parent_id: traceRootId ?? undefined,
      node_id: node.id,
      data: redactNodeForTrace(node),
    });
    return id;
  };

  const checkpointStore = options.checkpoint_store;
  let checkpointExtensions: Record<string, unknown> | undefined;
  if (checkpointStore && resume) {
    const loaded = await checkpointStore.load();
    if (loaded && isCompatibleCheckpoint(plan, loaded)) {
      restoreRuntime(ctx, loaded.runtime);
      for (const id of loaded.completed_node_ids) completed.add(id);
      for (const [id, st] of Object.entries(loaded.poll_state_by_node_id ?? {})) {
        pollStateByNodeId.set(id, st);
      }
      checkpointExtensions = isRecord(loaded.extensions) ? { ...loaded.extensions } : undefined;
    }
  }

  const maybeRecord = (ev: EngineEvent) => {
    if (options.include_events_in_checkpoint) eventsForCheckpoint.push(ev);
  };

  if (traceSink && traceRootId && traceRunId) {
    await traceAppend({
      kind: 'root',
      id: traceRootId,
      data: { plan: redactPlanForTrace(plan) },
    });
  }

  const saveCheckpoint = async () => {
    if (!checkpointStore) return;
    const checkpoint: EngineCheckpoint = {
      schema: 'ais-engine-checkpoint/0.0.2',
      created_at: new Date().toISOString(),
      plan,
      runtime: cloneRuntime(ctx.runtime),
      completed_node_ids: Array.from(completed),
      poll_state_by_node_id: Object.fromEntries(pollStateByNodeId.entries()),
      paused_by_node_id: Object.fromEntries(pausedByNodeId.entries()),
      events: options.include_events_in_checkpoint ? eventsForCheckpoint.slice() : undefined,
      extensions: checkpointExtensions,
    };
    await checkpointStore.save(checkpoint);
    const ev: EngineEvent = { type: 'checkpoint_saved', checkpoint };
    maybeRecord(ev);
    if (traceSink) {
      await traceAppend({ kind: 'event', id: randomUUID(), parent_id: traceRootId ?? undefined, data: redactEvent(ev) });
    }
    return ev;
  };

  const planReady: EngineEvent = { type: 'plan_ready', plan };
  maybeRecord(planReady);
  if (traceSink) {
    await traceAppend({ kind: 'event', id: randomUUID(), parent_id: traceRootId ?? undefined, data: redactEvent(planReady) });
  }
  yield planReady;

  // Running executor promises
  const running = new Map<
    string,
    {
      node: ExecutionPlanNode;
      io: NodeIoKind;
      promise: Promise<unknown>;
    }
  >();

  const inFlightByChain = new Map<string, { read: number; write: number }>();
  const inc = (chain: string, io: NodeIoKind) => {
    const cur = inFlightByChain.get(chain) ?? { read: 0, write: 0 };
    cur[io]++;
    inFlightByChain.set(chain, cur);
  };
  const dec = (chain: string, io: NodeIoKind) => {
    const cur = inFlightByChain.get(chain);
    if (!cur) return;
    cur[io] = Math.max(0, cur[io] - 1);
  };

  const isDone = () => completed.size >= plan.nodes.length;

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const nextPollWakeAtMs = (): number | null => {
    const now = Date.now();
    let min: number | null = null;
    for (const st of pollStateByNodeId.values()) {
      if (typeof st.next_attempt_at_ms !== 'number') continue;
      if (st.next_attempt_at_ms <= now) continue;
      if (min === null || st.next_attempt_at_ms < min) min = st.next_attempt_at_ms;
    }
    return min;
  };

  while (!isDone()) {
    let progressed = false;
    const nowMs = Date.now();

    // Try to start as many eligible nodes as we can.
    for (const node of plan.nodes) {
      if (isDone()) break;
      if (completed.has(node.id)) continue;
      if (running.has(node.id)) continue;
      if (pausedByNodeId.has(node.id)) continue;
      if (!depsSatisfied(node, completed)) continue;

      const pollState = pollStateByNodeId.get(node.id);
      if (pollState && typeof pollState.next_attempt_at_ms === 'number' && nowMs < pollState.next_attempt_at_ms) {
        continue;
      }

      const io = classifyIo(node);
      if (!canStart(node.chain, io, running.size, maxConcurrency, options.per_chain, inFlightByChain)) continue;

      // Readiness + solver (sequential, deterministic)
      const evalOpts = options.detect ? { detect: options.detect } : undefined;
      let readiness = options.detect ? await getNodeReadinessAsync(node, ctx, evalOpts) : getNodeReadiness(node, ctx);

      if (readiness.state === 'skipped') {
        const ev: EngineEvent = { type: 'skipped', node, reason: 'condition=false' };
        maybeRecord(ev);
        if (traceSink) {
          const span = await ensureNodeSpan(node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(ev) });
        }
        yield ev;
        completed.add(node.id);
        progressed = true;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        continue;
      }

      if (readiness.state === 'blocked') {
        const ev: EngineEvent = { type: 'node_blocked', node, readiness };
        maybeRecord(ev);
        if (traceSink) {
          const span = await ensureNodeSpan(node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(ev) });
        }
        yield ev;

        const solved = await options.solver.solve(node, readiness, ctx);
        if (solved.patches && solved.patches.length > 0) {
          applyRuntimePatches(ctx, solved.patches);
          const applied: EngineEvent = { type: 'solver_applied', node, patches: solved.patches };
          maybeRecord(applied);
          if (traceSink) {
            const span = await ensureNodeSpan(node);
            await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(applied) });
          }
          yield applied;
          const ck = await saveCheckpoint();
          if (ck) yield ck;
        }

        if (solved.need_user_confirm) {
          const enrichedDetails = enrichNeedUserConfirmDetails({
            node,
            reason: solved.need_user_confirm.reason,
            details: solved.need_user_confirm.details,
          });
          const need: EngineEvent = {
            type: 'need_user_confirm',
            node,
            reason: solved.need_user_confirm.reason,
            details: enrichedDetails,
          };
          maybeRecord(need);
          if (traceSink) {
            const span = await ensureNodeSpan(node);
            await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(need) });
          }
          yield need;
          pausedByNodeId.set(node.id, {
            reason: solved.need_user_confirm.reason,
            details: enrichedDetails,
            paused_at_ms: Date.now(),
          });
          const ck = await saveCheckpoint();
          if (ck) yield ck;
          progressed = true;
          continue;
        }

        if (solved.cannot_solve) {
          const errEv: EngineEvent = {
            type: 'error',
            node,
            error: new Error(solved.cannot_solve.reason),
            retryable: false,
          };
          maybeRecord(errEv);
          if (traceSink) {
            const span = await ensureNodeSpan(node);
            await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(errEv) });
          }
          yield errEv;
          const ck = await saveCheckpoint();
          if (ck) yield ck;
          if (stopOnError) return;
          continue;
        }

        readiness = options.detect ? await getNodeReadinessAsync(node, ctx, evalOpts) : getNodeReadiness(node, ctx);
        if (readiness.state !== 'ready') {
          const paused: EngineEvent = {
            type: 'node_paused',
            node,
            reason: 'node_blocked',
            details: readiness,
          };
          maybeRecord(paused);
          if (traceSink) {
            const span = await ensureNodeSpan(node);
            await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(paused) });
          }
          yield paused;
          pausedByNodeId.set(node.id, { reason: paused.reason, details: paused.details, paused_at_ms: Date.now() });
          const ck = await saveCheckpoint();
          if (ck) yield ck;
          progressed = true;
          continue;
        }
      }

      const readyEv: EngineEvent = { type: 'node_ready', node };
      maybeRecord(readyEv);
      if (traceSink) {
        const span = await ensureNodeSpan(node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(readyEv) });
      }
      yield readyEv;

      const executor = pickExecutor(node, options.executors);
      if (!executor) {
        const errEv: EngineEvent = {
          type: 'error',
          node,
          error: new Error(`No executor supports node (chain=${node.chain}, type=${node.execution.type})`),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      const existingPoll = pollStateByNodeId.get(node.id);
      if (existingPoll && existingPoll.next_attempt_at_ms !== undefined) {
        pollStateByNodeId.set(node.id, { ...existingPoll, next_attempt_at_ms: undefined });
      }

      running.set(node.id, {
        node,
        io,
        promise: Promise.resolve(executor.execute(node, ctx, { resolved_params: readiness.resolved_params, detect: options.detect })),
      });
      inc(node.chain, io);
      progressed = true;
    }

    const wakeAt = nextPollWakeAtMs();

    // No runnable nodes were started and nothing is running.
    if (running.size === 0) {
      if (wakeAt !== null) {
        const delay = Math.max(0, wakeAt - Date.now());
        if (delay > 0) await sleep(delay);
        continue;
      }

      if (pausedByNodeId.size > 0) {
        const paused: EngineEvent = {
          type: 'engine_paused',
          paused: Array.from(pausedByNodeId.entries()).map(([id, st]) => {
            const node = plan.nodes.find((n) => n.id === id)!;
            return { node, reason: st.reason, details: st.details };
          }),
        };
        maybeRecord(paused);
        if (traceSink) {
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: traceRootId ?? undefined, data: redactEvent(paused) });
        }
        yield paused;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        return;
      }

      if (!progressed) {
        const errEv: EngineEvent = {
          type: 'error',
          error: new Error('Engine deadlock: no runnable nodes (deps unsatisfied or permanently blocked)'),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: traceRootId ?? undefined, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        return;
      }
      continue;
    }

    // Wait for the next executor completion, but wake up for poll timers.
    const settled = await (async () => {
      const settlePromise = Promise.race(
        Array.from(running.entries()).map(async ([id, r]) => {
          try {
            const result = await r.promise;
            return { kind: 'settled' as const, id, node: r.node, io: r.io, ok: true as const, result };
          } catch (e) {
            return { kind: 'settled' as const, id, node: r.node, io: r.io, ok: false as const, error: e };
          }
        })
      );

      if (wakeAt === null) return await settlePromise;
      const delay = Math.max(0, wakeAt - Date.now());
      if (delay === 0) return await settlePromise;
      const wakePromise = sleep(delay).then(() => ({ kind: 'wake' as const }));

      return await Promise.race([settlePromise, wakePromise]);
    })();

    if (settled.kind === 'wake') {
      continue;
    }

    running.delete(settled.id);
    dec(settled.node.chain, settled.io);

    if (!settled.ok) {
      const errEv: EngineEvent = {
        type: 'error',
        node: settled.node,
        error: settled.error instanceof Error ? settled.error : new Error(String(settled.error)),
        retryable: true,
      };
      maybeRecord(errEv);
      if (traceSink) {
        const span = await ensureNodeSpan(settled.node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
      }
      yield errEv;
      const ck = await saveCheckpoint();
      if (ck) yield ck;
      if (stopOnError) return;
      continue;
    }

    const execResult = settled.result as any;

    if (execResult?.need_user_confirm) {
      const enrichedDetails = enrichNeedUserConfirmDetails({
        node: settled.node,
        reason: execResult.need_user_confirm.reason,
        details: execResult.need_user_confirm.details,
      });
      const need: EngineEvent = {
        type: 'need_user_confirm',
        node: settled.node,
        reason: execResult.need_user_confirm.reason,
        details: enrichedDetails,
      };
      maybeRecord(need);
      if (traceSink) {
        const span = await ensureNodeSpan(settled.node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(need) });
      }
      yield need;
      pausedByNodeId.set(settled.node.id, {
        reason: execResult.need_user_confirm.reason,
        details: enrichedDetails,
        paused_at_ms: Date.now(),
      });
      const ck = await saveCheckpoint();
      if (ck) yield ck;
      continue;
    }

    if (execResult?.patches && execResult.patches.length > 0) {
      applyRuntimePatches(ctx, execResult.patches);
    }

    // Event mapping
    if (settled.node.execution.type === 'evm_read') {
      const ev: EngineEvent = { type: 'query_result', node: settled.node, outputs: execResult.outputs ?? {} };
      maybeRecord(ev);
      if (traceSink) {
        const span = await ensureNodeSpan(settled.node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(ev) });
      }
      yield ev;
    } else if (settled.node.execution.type === 'evm_call') {
      const txHash = execResult.outputs?.tx_hash;
      if (typeof txHash === 'string') {
        const sent: EngineEvent = { type: 'tx_sent', node: settled.node, tx_hash: txHash };
        maybeRecord(sent);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(sent) });
        }
        yield sent;
      }
      if (execResult.outputs?.receipt) {
        const conf: EngineEvent = { type: 'tx_confirmed', node: settled.node, receipt: execResult.outputs.receipt };
        maybeRecord(conf);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(conf) });
        }
        yield conf;
      }
    } else if (settled.node.execution.type === 'solana_instruction') {
      const sig = execResult.outputs?.signature;
      if (typeof sig === 'string') {
        const sent: EngineEvent = { type: 'tx_sent', node: settled.node, tx_hash: sig };
        maybeRecord(sent);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(sent) });
        }
        yield sent;
      }
      if (execResult.outputs?.confirmation) {
        const conf: EngineEvent = { type: 'tx_confirmed', node: settled.node, receipt: execResult.outputs.confirmation };
        maybeRecord(conf);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(conf) });
        }
        yield conf;
      }
    } else {
      const ev: EngineEvent = { type: 'query_result', node: settled.node, outputs: execResult.outputs ?? {} };
      maybeRecord(ev);
      if (traceSink) {
        const span = await ensureNodeSpan(settled.node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(ev) });
      }
      yield ev;
    }

    if (settled.node.assert) {
      let assertValue: unknown;
      try {
        assertValue = options.detect
          ? await evaluateValueRefAsync(settled.node.assert as any, ctx, { detect: options.detect })
          : evaluateValueRef(settled.node.assert as any, ctx);
      } catch (e) {
        const err = e instanceof ValueRefEvalError ? e : new Error(String(e));
        const errEv: EngineEvent = { type: 'error', node: settled.node, error: err, retryable: false };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        pausedByNodeId.set(settled.node.id, {
          reason: 'assert evaluation failed',
          details: { assert: formatValueRefForMessage(settled.node.assert), error: err.message },
          paused_at_ms: Date.now(),
        });
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      if (!Boolean(assertValue)) {
        const expr = formatValueRefForMessage(settled.node.assert);
        const message = settled.node.assert_message ?? `Node assert failed: ${expr}`;
        const errEv: EngineEvent = {
          type: 'error',
          node: settled.node,
          error: new Error(message),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        pausedByNodeId.set(settled.node.id, {
          reason: 'assert failed',
          details: { assert: expr, value: assertValue },
          paused_at_ms: Date.now(),
        });
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }
    }

    if (settled.node.until) {
      if (classifyIo(settled.node) !== 'read') {
        const errEv: EngineEvent = {
          type: 'error',
          node: settled.node,
          error: new Error('until/retry is only supported for read nodes'),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      if (!settled.node.retry) {
        const errEv: EngineEvent = {
          type: 'error',
          node: settled.node,
          error: new Error('Workflow node has until but no retry policy'),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      const prev = pollStateByNodeId.get(settled.node.id);
      const startedAt = prev?.started_at_ms ?? Date.now();
      const attempts = (prev?.attempts ?? 0) + 1;

      let untilValue: unknown;
      try {
        untilValue = options.detect
          ? await evaluateValueRefAsync(settled.node.until as any, ctx, { detect: options.detect })
          : evaluateValueRef(settled.node.until as any, ctx);
      } catch (e) {
        const err = e instanceof ValueRefEvalError ? e : new Error(String(e));
        const errEv: EngineEvent = { type: 'error', node: settled.node, error: err, retryable: false };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      if (Boolean(untilValue)) {
        pollStateByNodeId.delete(settled.node.id);
        completed.add(settled.node.id);
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        continue;
      }

      if (settled.node.timeout_ms !== undefined) {
        const elapsed = Date.now() - startedAt;
        if (elapsed >= settled.node.timeout_ms) {
          const errEv: EngineEvent = {
            type: 'error',
            node: settled.node,
            error: new Error(
              `until timeout exceeded (elapsed_ms=${elapsed}, timeout_ms=${settled.node.timeout_ms})`
            ),
            retryable: false,
          };
          maybeRecord(errEv);
          if (traceSink) {
            const span = await ensureNodeSpan(settled.node);
            await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
          }
          yield errEv;
          const ck = await saveCheckpoint();
          if (ck) yield ck;
          if (stopOnError) return;
          continue;
        }
      }

      if (settled.node.retry.max_attempts !== undefined && attempts >= settled.node.retry.max_attempts) {
        const errEv: EngineEvent = {
          type: 'error',
          node: settled.node,
          error: new Error(
            `until max_attempts exceeded (attempts=${attempts}, max_attempts=${settled.node.retry.max_attempts})`
          ),
          retryable: false,
        };
        maybeRecord(errEv);
        if (traceSink) {
          const span = await ensureNodeSpan(settled.node);
          await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(errEv) });
        }
        yield errEv;
        const ck = await saveCheckpoint();
        if (ck) yield ck;
        if (stopOnError) return;
        continue;
      }

      const nextAttemptAt = Date.now() + settled.node.retry.interval_ms;
      pollStateByNodeId.set(settled.node.id, {
        attempts,
        started_at_ms: startedAt,
        next_attempt_at_ms: nextAttemptAt,
      });
      const waiting: EngineEvent = {
        type: 'node_waiting',
        node: settled.node,
        attempts,
        next_attempt_at_ms: nextAttemptAt,
      };
      maybeRecord(waiting);
      if (traceSink) {
        const span = await ensureNodeSpan(settled.node);
        await traceAppend({ kind: 'event', id: randomUUID(), parent_id: span, node_id: settled.node.id, data: redactEvent(waiting) });
      }
      yield waiting;
      const ck = await saveCheckpoint();
      if (ck) yield ck;
      continue;
    }

    pollStateByNodeId.delete(settled.node.id);
    completed.add(settled.node.id);
    const ck = await saveCheckpoint();
    if (ck) yield ck;
  }
}

function enrichNeedUserConfirmDetails(args: {
  node: ExecutionPlanNode;
  reason: string;
  details?: unknown;
}): unknown {
  const { node, reason } = args;
  const base = isRecord(args.details) ? { ...(args.details as Record<string, unknown>) } : { details: args.details };

  // Avoid re-hashing if already present.
  if (!base.confirmation_summary) {
    const summary = summarizeNeedUserConfirm({ node, reason, details: args.details });
    base.confirmation_summary = summary;
    base.confirmation_hash = summary.hash;
  }
  return base;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function classifyIo(node: ExecutionPlanNode): NodeIoKind {
  if (
    node.execution.type === 'evm_read' ||
    node.execution.type === 'evm_rpc' ||
    node.execution.type === 'evm_multiread' ||
    node.execution.type === 'solana_read'
  ) {
    return 'read';
  }
  return 'write';
}

function depsSatisfied(node: ExecutionPlanNode, completed: Set<string>): boolean {
  const deps = node.deps ?? [];
  for (const d of deps) {
    if (!completed.has(d)) return false;
  }
  return true;
}

function pickExecutor(node: ExecutionPlanNode, executors: Executor[]): Executor | null {
  for (const ex of executors) {
    if (ex.supports(node)) return ex;
  }
  return null;
}

function canStart(
  chain: string,
  io: NodeIoKind,
  runningTotal: number,
  maxConcurrency: number,
  perChain: RunPlanOptions['per_chain'],
  inFlightByChain: Map<string, { read: number; write: number }>
): boolean {
  if (runningTotal >= maxConcurrency) return false;

  const defaults = { max_read_concurrency: 8, max_write_concurrency: 1 };
  const cfg = perChain?.[chain] ?? defaults;

  const cur = inFlightByChain.get(chain) ?? { read: 0, write: 0 };
  if (io === 'read') {
    const limit = cfg.max_read_concurrency ?? defaults.max_read_concurrency;
    return cur.read < limit;
  }
  const limit = cfg.max_write_concurrency ?? defaults.max_write_concurrency;
  return cur.write < limit;
}

function isCompatibleCheckpoint(plan: ExecutionPlan, checkpoint: EngineCheckpoint): boolean {
  if (checkpoint.schema !== 'ais-engine-checkpoint/0.0.2') return false;
  if (checkpoint.plan?.schema !== plan.schema) return false;
  const a = plan.nodes.map((n) => n.id);
  const b = checkpoint.plan.nodes.map((n) => n.id);
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function restoreRuntime(ctx: ResolverContext, runtime: unknown): void {
  if (!runtime || typeof runtime !== 'object') return;
  ctx.runtime = cloneRuntime(runtime as any) as any;
}

function cloneRuntime<T>(value: T): T {
  // Prefer structuredClone (supports BigInt/Uint8Array)
  if (typeof (globalThis as any).structuredClone === 'function') {
    return (globalThis as any).structuredClone(value);
  }
  return deepClone(value);
}

function deepClone<T>(value: T): T {
  if (value === null || value === undefined) return value;
  const t = typeof value;
  if (t === 'string' || t === 'number' || t === 'boolean' || t === 'bigint') return value;
  if (value instanceof Uint8Array) return new Uint8Array(value) as any;
  if (Array.isArray(value)) return value.map((v) => deepClone(v)) as any;
  if (t === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as any)) out[k] = deepClone(v);
    return out as any;
  }
  return value;
}

function formatValueRefForMessage(ref: unknown): string {
  if (!ref || typeof ref !== 'object') return '<invalid assert>';
  const v = ref as Record<string, unknown>;
  if (typeof v.cel === 'string') return `cel(${v.cel})`;
  if (typeof v.ref === 'string') return `ref(${v.ref})`;
  if ('lit' in v) return `lit(${String(v.lit)})`;
  return '<assert>';
}
