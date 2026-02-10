import type { Solver, SolverResult } from '../types.js';
import type { ResolverContext } from '../../resolver/index.js';
import { parseSkillRef, resolveProtocolRef } from '../../resolver/index.js';
import type { ExecutionPlanNode, NodeReadinessResult } from '../../execution/index.js';
import type { RuntimePatch } from '../patch.js';

export interface SolverOptions {
  /**
   * When true, automatically fills `runtime.contracts` from the protocol
   * deployment for the node's chain when `missing_refs` includes `contracts.*`.
   */
  auto_fill_contracts?: boolean;
}

export function createSolver(options: SolverOptions = {}): Solver {
  const autoFillContracts = options.auto_fill_contracts ?? true;

  return {
    solve(node: ExecutionPlanNode, readiness: NodeReadinessResult, ctx: ResolverContext): SolverResult {
      if (readiness.state !== 'blocked') return { patches: [] };

      // Detect handling
      if (readiness.needs_detect) {
        return {
          need_user_confirm: {
            reason: 'detect resolution required',
            details: { node_id: node.id, errors: readiness.errors },
          },
        };
      }

      const missing = readiness.missing_refs ?? [];
      const patches: RuntimePatch[] = [];

      // Auto-fill protocol contracts
      if (autoFillContracts && missing.some((m) => m.startsWith('contracts.'))) {
        const filled = fillContractsForNode(node, ctx);
        if (filled) patches.push(filled);
      }

      // Recompute what is still missing after our auto-fills. The engine will
      // re-run readiness, but we can proactively guide UX here.
      const remaining = missing.filter((m) => {
        if (m.startsWith('contracts.') && patches.some((p) => p.path === 'contracts')) return false;
        return true;
      });

      if (remaining.length > 0) {
        return {
          patches,
          need_user_confirm: {
            reason: 'missing runtime inputs',
            details: { missing_refs: remaining, node_id: node.id },
          },
        };
      }

      return { patches };
    },
  };
}

/**
 * Built-in solver instance (minimal).
 *
 * Use `createSolver()` if you need custom behavior.
 */
export const solver: Solver = createSolver();

function fillContractsForNode(node: ExecutionPlanNode, ctx: ResolverContext): RuntimePatch | null {
  const skill = node.source?.skill;
  if (!skill) return null;

  const { protocol } = parseSkillRef(skill);
  const spec = resolveProtocolRef(ctx, protocol);
  if (!spec) return null;

  const deployment = spec.deployments.find((d) => d.chain === node.chain);
  if (!deployment) return null;

  return { op: 'merge', path: 'contracts', value: deployment.contracts };
}

