type Solver = {
  solve(node: any, readiness: any, ctx: any): Promise<any> | any;
};

export function wrapSolverWithCalculatedFields(args: {
  sdk: any;
  inner: Solver;
  detect?: any;
}): Solver {
  const { sdk, inner, detect } = args;

  return {
    async solve(node: any, readiness: any, ctx: any): Promise<any> {
      const base = await inner.solve(node, readiness, ctx);
      const basePatches = Array.isArray(base?.patches) ? base.patches : [];

      // If the base solver already wants user confirmation for something else,
      // don't try to be clever (but we can still add patches if we can).
      const baseNeed = base?.need_user_confirm;
      const baseCannot = base?.cannot_solve;

      let patches = basePatches.slice();

      if (patches.length > 0) {
        // Apply early so our computed fields can see updated runtime (idempotent merge patches).
        sdk.applyRuntimePatches(ctx, patches);
      }

      const missing = Array.isArray(readiness?.missing_refs) ? readiness.missing_refs : [];
      const shouldCompute = readiness?.state === 'blocked' && missing.some((m: any) => String(m).startsWith('calculated.'));
      if (!shouldCompute) return base;

      const calcRes = await computeCalculatedFieldPatches(sdk, node, ctx, readiness?.resolved_params ?? {}, detect);
      if (calcRes.kind === 'need_user_confirm') {
        // Preserve base patches if any; prefer calculated_fields failure reason.
        return { patches, need_user_confirm: calcRes.need_user_confirm };
      }
      if (calcRes.patches.length > 0) {
        patches = patches.concat(calcRes.patches);
        sdk.applyRuntimePatches(ctx, calcRes.patches);
      }

      // Re-check readiness after our patches: avoid pausing if we actually fixed it.
      const evalOpts = detect ? { detect } : undefined;
      const r2 = detect ? await sdk.getNodeReadinessAsync(node, ctx, evalOpts) : sdk.getNodeReadiness(node, ctx, evalOpts);
      if (r2?.state === 'ready' || r2?.state === 'skipped') {
        return { patches };
      }

      if (baseCannot) return { patches, cannot_solve: baseCannot };
      if (baseNeed) return { patches, need_user_confirm: baseNeed };

      const missing2 = Array.isArray(r2?.missing_refs) ? r2.missing_refs : [];
      const errors2 = Array.isArray(r2?.errors) ? r2.errors : [];
      return {
        patches,
        need_user_confirm: {
          reason: errors2.length > 0 ? 'readiness errors remain' : 'missing runtime inputs',
          details: { node_id: node?.id, missing_refs: missing2, errors: errors2 },
        },
      };
    },
  };
}

async function computeCalculatedFieldPatches(
  sdk: any,
  node: any,
  ctx: any,
  resolvedParams: Record<string, unknown>,
  detect: any | undefined
): Promise<
  | { kind: 'ok'; patches: any[] }
  | { kind: 'need_user_confirm'; need_user_confirm: { reason: string; details?: unknown } }
> {
  const skill = node?.source?.skill;
  const actionId = node?.source?.action;
  if (typeof skill !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
    return { kind: 'ok', patches: [] };
  }

  const resolved = sdk.resolveAction(ctx, `${skill}/${actionId}`);
  if (!resolved) {
    return {
      kind: 'need_user_confirm',
      need_user_confirm: {
        reason: 'action not found for calculated_fields (resolveAction failed)',
        details: { skill, action: actionId, node_id: node?.id },
      },
    };
  }

  const req = resolved.action?.requires_queries;
  if (Array.isArray(req) && req.length > 0) {
    const missingQueries = req.filter((q: any) => typeof q === 'string' && ctx?.runtime?.query?.[q] === undefined);
    if (missingQueries.length > 0) {
      return {
        kind: 'need_user_confirm',
        need_user_confirm: {
          reason: 'missing required queries for action (needed for calculated_fields)',
          details: { node_id: node?.id, action_ref: `${skill}/${actionId}`, missing_queries: missingQueries },
        },
      };
    }
  }

  const calculated = resolved.action?.calculated_fields;
  if (!calculated || typeof calculated !== 'object') {
    return { kind: 'ok', patches: [] };
  }

  const order = topoOrderCalculatedFields(calculated as any);
  const computed: Record<string, unknown> = {};

  for (const name of order) {
    const def = (calculated as any)[name];
    const expr = def?.expr;
    if (!expr) continue;
    try {
      const evalOpts = detect
        ? { root_overrides: { params: resolvedParams }, detect }
        : { root_overrides: { params: resolvedParams } };
      const v = await sdk.evaluateValueRefAsync(expr, ctx, evalOpts);
      computed[name] = v;
    } catch (e) {
      const msg = (e as Error)?.message ?? String(e);
      return {
        kind: 'need_user_confirm',
        need_user_confirm: {
          reason: 'calculated_fields evaluation failed',
          details: { node_id: node?.id, action_ref: `${skill}/${actionId}`, field: name, error: msg },
        },
      };
    }
  }

  const patches = [
    { op: 'merge', path: 'calculated', value: computed },
    { op: 'merge', path: `nodes.${String(node?.id ?? '')}.calculated`, value: computed },
  ];
  return { kind: 'ok', patches };
}

function topoOrderCalculatedFields(calculated: Record<string, { inputs?: string[] }>): string[] {
  const names = Object.keys(calculated);
  const originalIndex = new Map(names.map((n, i) => [n, i] as const));

  const depsByName = new Map<string, Set<string>>();
  const outgoing = new Map<string, Set<string>>();
  const inDegree = new Map<string, number>();

  for (const n of names) {
    depsByName.set(n, new Set());
    outgoing.set(n, new Set());
    inDegree.set(n, 0);
  }

  for (const [name, def] of Object.entries(calculated)) {
    for (const inp of def?.inputs ?? []) {
      if (typeof inp !== 'string') continue;
      const dep = extractCalculatedDep(inp);
      if (!dep) continue;
      if (!depsByName.has(name) || !depsByName.has(dep)) continue;
      depsByName.get(name)!.add(dep);
    }
  }

  for (const [name, deps] of depsByName.entries()) {
    for (const dep of deps) {
      outgoing.get(dep)!.add(name);
      inDegree.set(name, (inDegree.get(name) ?? 0) + 1);
    }
  }

  const available: string[] = [];
  for (const n of names) if ((inDegree.get(n) ?? 0) === 0) available.push(n);
  const sortAvail = () =>
    available.sort((a, b) => {
      const ia = originalIndex.get(a) ?? 0;
      const ib = originalIndex.get(b) ?? 0;
      if (ia !== ib) return ia - ib;
      return a.localeCompare(b);
    });
  sortAvail();

  const ordered: string[] = [];
  while (available.length > 0) {
    const n = available.shift()!;
    ordered.push(n);
    for (const nxt of outgoing.get(n) ?? []) {
      const deg = (inDegree.get(nxt) ?? 0) - 1;
      inDegree.set(nxt, deg);
      if (deg === 0) available.push(nxt);
    }
    sortAvail();
  }

  if (ordered.length !== names.length) {
    return names.slice().sort((a, b) => (originalIndex.get(a) ?? 0) - (originalIndex.get(b) ?? 0));
  }
  return ordered;
}

function extractCalculatedDep(input: string): string | null {
  if (!input.startsWith('calculated.')) return null;
  const rest = input.slice('calculated.'.length);
  const first = rest.split('.', 1)[0];
  return first && /^[A-Za-z_][A-Za-z0-9_]*$/.test(first) ? first : null;
}

