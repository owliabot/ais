import type {
  RunnerContext,
  RunnerDetectResolver,
  RunnerPatch,
  RunnerPlanNode,
  RunnerReadiness,
  RunnerSdkModule,
  RunnerSolver,
  RunnerSolverResult,
} from './types.js';

type NeedUserConfirm = NonNullable<RunnerSolverResult['need_user_confirm']>;
type CannotSolve = NonNullable<RunnerSolverResult['cannot_solve']>;
type ResolvedAction = NonNullable<ReturnType<RunnerSdkModule['resolveAction']>>;
type CalculatedFields = NonNullable<ResolvedAction['action']['calculated_fields']>;

type SolverWrapperSdk = Pick<
  RunnerSdkModule,
  | 'resolveAction'
  | 'applyRuntimePatches'
  | 'evaluateValueRefAsync'
  | 'getNodeReadiness'
  | 'getNodeReadinessAsync'
>;

type CalculatedFieldPatchResult =
  | { kind: 'ok'; patches: RunnerPatch[] }
  | { kind: 'need_user_confirm'; need_user_confirm: NeedUserConfirm };

export function wrapSolverWithCalculatedFields(args: {
  sdk: SolverWrapperSdk;
  inner: RunnerSolver;
  detect?: RunnerDetectResolver;
}): RunnerSolver {
  const { sdk, inner, detect } = args;

  return {
    async solve(node: RunnerPlanNode, readiness: RunnerReadiness, ctx: RunnerContext): Promise<RunnerSolverResult> {
      const base = await inner.solve(node, readiness, ctx);
      const basePatches = Array.isArray(base?.patches) ? base.patches : [];
      const baseNeed = base?.need_user_confirm;
      const baseCannot = base?.cannot_solve;

      let patches = basePatches.slice();

      if (patches.length > 0) {
        sdk.applyRuntimePatches(ctx, patches);
      }

      const shouldCompute =
        readiness.state === 'blocked' &&
        readiness.missing_refs.some((missingRef) => missingRef.startsWith('calculated.'));
      if (!shouldCompute) return base;

      const calcRes = await computeCalculatedFieldPatches(
        sdk,
        node,
        ctx,
        readiness.resolved_params ?? {},
        detect
      );
      if (calcRes.kind === 'need_user_confirm') {
        return { patches, need_user_confirm: calcRes.need_user_confirm };
      }

      if (calcRes.patches.length > 0) {
        patches = patches.concat(calcRes.patches);
        sdk.applyRuntimePatches(ctx, calcRes.patches);
      }

      const evalOpts = detect ? { detect } : undefined;
      const r2 = detect
        ? await sdk.getNodeReadinessAsync(node, ctx, evalOpts)
        : sdk.getNodeReadiness(node, ctx, evalOpts);
      if (r2.state === 'ready' || r2.state === 'skipped') {
        return { patches };
      }

      if (baseCannot) return { patches, cannot_solve: baseCannot };
      if (baseNeed) return { patches, need_user_confirm: baseNeed };

      return {
        patches,
        need_user_confirm: {
          reason: r2.errors.length > 0 ? 'readiness errors remain' : 'missing runtime inputs',
          details: { node_id: node.id, missing_refs: r2.missing_refs, errors: r2.errors },
        },
      };
    },
  };
}

async function computeCalculatedFieldPatches(
  sdk: SolverWrapperSdk,
  node: RunnerPlanNode,
  ctx: RunnerContext,
  resolvedParams: Record<string, unknown>,
  detect: RunnerDetectResolver | undefined
): Promise<CalculatedFieldPatchResult> {
  const protocolRef = node.source?.protocol;
  const actionId = node.source?.action;
  if (typeof protocolRef !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
    return { kind: 'ok', patches: [] };
  }

  const resolved = sdk.resolveAction(ctx, `${protocolRef}/${actionId}`);
  if (!resolved) {
    return {
      kind: 'need_user_confirm',
      need_user_confirm: {
        reason: 'action not found for calculated_fields (resolveAction failed)',
        details: { protocol: protocolRef, action: actionId, node_id: node.id },
      },
    };
  }

  const requiresQueries = resolved.action?.requires_queries;
  if (Array.isArray(requiresQueries) && requiresQueries.length > 0) {
    const missingQueries = requiresQueries.filter((queryName) => ctx.runtime.query?.[queryName] === undefined);
    if (missingQueries.length > 0) {
      return {
        kind: 'need_user_confirm',
        need_user_confirm: {
          reason: 'missing required queries for action (needed for calculated_fields)',
          details: { node_id: node.id, action_ref: `${protocolRef}/${actionId}`, missing_queries: missingQueries },
        },
      };
    }
  }

  const calculated = resolved.action?.calculated_fields;
  if (!calculated || typeof calculated !== 'object') {
    return { kind: 'ok', patches: [] };
  }

  const order = topoOrderCalculatedFields(calculated);
  const computed: Record<string, unknown> = {};

  for (const name of order) {
    const def = calculated[name];
    const expr = def?.expr;
    if (!expr) continue;

    try {
      const evalOpts = detect
        ? { root_overrides: { params: resolvedParams }, detect }
        : { root_overrides: { params: resolvedParams } };
      const value = await sdk.evaluateValueRefAsync(expr, ctx, evalOpts);
      computed[name] = value;
    } catch (error) {
      const message = (error as Error)?.message ?? String(error);
      return {
        kind: 'need_user_confirm',
        need_user_confirm: {
          reason: 'calculated_fields evaluation failed',
          details: { node_id: node.id, action_ref: `${protocolRef}/${actionId}`, field: name, error: message },
        },
      };
    }
  }

  const patches: RunnerPatch[] = [
    { op: 'merge', path: 'calculated', value: computed },
    { op: 'merge', path: `nodes.${node.id}.calculated`, value: computed },
  ];
  return { kind: 'ok', patches };
}

function topoOrderCalculatedFields(calculated: CalculatedFields): string[] {
  const names = Object.keys(calculated);
  const originalIndex = new Map(names.map((name, index) => [name, index] as const));

  const depsByName = new Map<string, Set<string>>();
  const outgoing = new Map<string, Set<string>>();
  const inDegree = new Map<string, number>();

  for (const name of names) {
    depsByName.set(name, new Set());
    outgoing.set(name, new Set());
    inDegree.set(name, 0);
  }

  for (const [name, def] of Object.entries(calculated)) {
    for (const input of def?.inputs ?? []) {
      if (typeof input !== 'string') continue;
      const dep = extractCalculatedDep(input);
      if (!dep) continue;
      if (!depsByName.has(name) || !depsByName.has(dep)) continue;
      depsByName.get(name)?.add(dep);
    }
  }

  for (const [name, deps] of depsByName.entries()) {
    for (const dep of deps) {
      outgoing.get(dep)?.add(name);
      inDegree.set(name, (inDegree.get(name) ?? 0) + 1);
    }
  }

  const available: string[] = [];
  for (const name of names) if ((inDegree.get(name) ?? 0) === 0) available.push(name);
  const sortAvailable = () =>
    available.sort((left, right) => {
      const leftIndex = originalIndex.get(left) ?? 0;
      const rightIndex = originalIndex.get(right) ?? 0;
      if (leftIndex !== rightIndex) return leftIndex - rightIndex;
      return left.localeCompare(right);
    });
  sortAvailable();

  const ordered: string[] = [];
  while (available.length > 0) {
    const name = available.shift();
    if (!name) break;
    ordered.push(name);
    for (const next of outgoing.get(name) ?? []) {
      const deg = (inDegree.get(next) ?? 0) - 1;
      inDegree.set(next, deg);
      if (deg === 0) available.push(next);
    }
    sortAvailable();
  }

  if (ordered.length !== names.length) {
    return names.slice().sort((left, right) => (originalIndex.get(left) ?? 0) - (originalIndex.get(right) ?? 0));
  }
  return ordered;
}

function extractCalculatedDep(input: string): string | null {
  if (!input.startsWith('calculated.')) return null;
  const rest = input.slice('calculated.'.length);
  const first = rest.split('.', 1)[0];
  return first && /^[A-Za-z_][A-Za-z0-9_]*$/.test(first) ? first : null;
}
