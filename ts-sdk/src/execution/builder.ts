/**
 * Execution Builder (AIS 0.0.2)
 *
 * NOTE: The execution layer is being refactored to:
 * - ValueRef-based argument evaluation
 * - JSON ABI encoding (including tuples)
 * - Cross-chain composite steps (`steps[].execution`)
 *
 * For now, the SDK focuses on parsing/validation. Execution planning/building
 * will be implemented in follow-up tasks (see `docs/TODO.md`).
 */

import type { Action, Query, ProtocolSpec } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';

export interface TransactionRequest {
  to: string;
  data: string;
  value: bigint;
  chainId: number;
  stepId?: string;
  stepDescription?: string;
}

export interface BuildOptions {
  chain: string;
}

export interface BuildResult {
  success: true;
  transactions: TransactionRequest[];
  action: Action;
  resolvedParams: Record<string, unknown>;
  calculatedValues: Record<string, unknown>;
}

export interface BuildError {
  success: false;
  error: string;
  details?: unknown;
}

export type BuildOutput = BuildResult | BuildError;

export function buildTransaction(
  _protocol: ProtocolSpec,
  action: Action,
  _inputs: Record<string, unknown>,
  _ctx: ResolverContext,
  _options: BuildOptions
): BuildOutput {
  return {
    success: false,
    error: `Execution builder not implemented for AIS 0.0.2 yet (action: ${action.description})`,
  };
}

export function buildQuery(
  _protocol: ProtocolSpec,
  query: Query,
  _inputs: Record<string, unknown>,
  _ctx: ResolverContext,
  _options: BuildOptions
): BuildOutput {
  return {
    success: false,
    error: `Query builder not implemented for AIS 0.0.2 yet (query: ${query.description})`,
  };
}

export function buildWorkflowTransactions(
  _protocols: Map<string, ProtocolSpec>,
  _nodes: Array<{ skill: string; action?: string; query?: string; args?: Record<string, unknown> }>,
  _ctx: ResolverContext,
  _chain: string
): Array<BuildOutput & { nodeIndex: number }> {
  return [];
}
