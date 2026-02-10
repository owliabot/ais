import type { ExecutionPlan, ExecutionPlanNode, NodeReadinessResult } from '../execution/index.js';
import type { ResolverContext } from '../resolver/index.js';
import type { RuntimePatch } from './patch.js';
import type { DetectResolver } from '../resolver/value-ref.js';

export type EngineEvent =
  | { type: 'plan_ready'; plan: ExecutionPlan }
  | { type: 'node_ready'; node: ExecutionPlanNode }
  | { type: 'node_blocked'; node: ExecutionPlanNode; readiness: NodeReadinessResult }
  | { type: 'node_paused'; node: ExecutionPlanNode; reason: string; details?: unknown }
  | { type: 'solver_applied'; node: ExecutionPlanNode; patches: RuntimePatch[] }
  | { type: 'query_result'; node: ExecutionPlanNode; outputs: Record<string, unknown> }
  | { type: 'tx_prepared'; node: ExecutionPlanNode; tx: unknown }
  | { type: 'need_user_confirm'; node: ExecutionPlanNode; reason: string; details?: unknown }
  | { type: 'tx_sent'; node: ExecutionPlanNode; tx_hash: string }
  | { type: 'tx_confirmed'; node: ExecutionPlanNode; receipt: unknown }
  | { type: 'node_waiting'; node: ExecutionPlanNode; attempts: number; next_attempt_at_ms: number }
  | { type: 'engine_paused'; paused: Array<{ node: ExecutionPlanNode; reason: string; details?: unknown }> }
  | { type: 'skipped'; node: ExecutionPlanNode; reason: string }
  | { type: 'error'; node?: ExecutionPlanNode; error: Error; retryable?: boolean }
  | { type: 'checkpoint_saved'; checkpoint: EngineCheckpoint };

export interface NodePollState {
  attempts: number;
  started_at_ms: number;
  next_attempt_at_ms?: number;
}

export interface NodePauseState {
  reason: string;
  details?: unknown;
  paused_at_ms: number;
}

export interface EngineCheckpoint {
  schema: 'ais-engine-checkpoint/0.0.2';
  created_at: string;
  plan: ExecutionPlan;
  runtime: unknown;
  completed_node_ids: string[];
  poll_state_by_node_id?: Record<string, NodePollState>;
  paused_by_node_id?: Record<string, NodePauseState>;
  events?: EngineEvent[];
}

export interface CheckpointStore {
  load(): Promise<EngineCheckpoint | null>;
  save(checkpoint: EngineCheckpoint): Promise<void>;
}

export interface SolverResult {
  patches?: RuntimePatch[];
  need_user_confirm?: { reason: string; details?: unknown };
  cannot_solve?: { reason: string; details?: unknown };
}

export interface Solver {
  solve(
    node: ExecutionPlanNode,
    readiness: NodeReadinessResult,
    ctx: ResolverContext
  ): Promise<SolverResult> | SolverResult;
}

export interface ExecutorResult {
  patches?: RuntimePatch[];
  outputs?: Record<string, unknown>;
  telemetry?: Record<string, unknown>;
  need_user_confirm?: { reason: string; details?: unknown };
}

export interface Executor {
  supports(node: ExecutionPlanNode): boolean;
  execute(
    node: ExecutionPlanNode,
    ctx: ResolverContext,
    options?: { resolved_params?: Record<string, unknown>; detect?: DetectResolver }
  ): Promise<ExecutorResult> | ExecutorResult;
}
