type Executor = {
  supports(node: any): boolean;
  execute(node: any, ctx: any, options?: any): Promise<any> | any;
  destroy?: () => void | Promise<void>;
};

type SdkLike = {
  compileEvmExecution?: (exec: any, ctx: any, options: any) => any;
  solana?: { compileSolanaInstruction?: (exec: any, ctx: any, options: any) => any };
};

export class StrictSuccessExecutor implements Executor {
  constructor(private readonly inner: Executor) {}

  supports(node: any): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(node: any, ctx: any, options?: any): Promise<any> {
    const res = await this.inner.execute(node, ctx, options);
    const outputs = res?.outputs;
    if (!outputs || typeof outputs !== 'object') return res;

    // EVM: require receipt.status truthy when available.
    const receipt = (outputs as any).receipt;
    if (receipt && typeof receipt === 'object' && 'status' in receipt) {
      const st = (receipt as any).status;
      if (isEvmFailureStatus(st)) {
        throw new Error(`EVM receipt status indicates failure: status=${String(st)}`);
      }
    }

    // Solana: if confirmation includes err, treat as failure (defensive; executor usually throws earlier).
    const confirmation = (outputs as any).confirmation;
    const err = confirmation?.value?.err;
    if (err) {
      throw new Error(`Solana confirmation indicates failure: err=${JSON.stringify(err)}`);
    }

    return res;
  }
}

export class BroadcastGateExecutor implements Executor {
  constructor(
    private readonly sdk: SdkLike,
    private readonly inner: Executor,
    private readonly allowBroadcast: boolean
  ) {}

  supports(node: any): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(node: any, ctx: any, options?: any): Promise<any> {
    if (!this.allowBroadcast && classifyIo(node) === 'write') {
      const resolvedParams = options?.resolved_params ?? {};
      const details = compileWritePreview(this.sdk, node, ctx, resolvedParams);
      return {
        need_user_confirm: {
          reason: 'broadcast disabled (pass --broadcast to allow write execution)',
          details,
        },
      };
    }
    return await this.inner.execute(node, ctx, options);
  }
}

export class ActionPreflightExecutor implements Executor {
  constructor(
    private readonly sdk: any,
    private readonly inner: Executor
  ) {}

  supports(node: any): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(node: any, ctx: any, options?: any): Promise<any> {
    if (classifyIo(node) !== 'write') return await this.inner.execute(node, ctx, options);

    const skill = node?.source?.skill;
    const actionId = node?.source?.action;
    if (typeof skill === 'string' && typeof actionId === 'string' && actionId.length > 0) {
      const resolved = this.sdk.resolveAction(ctx, `${skill}/${actionId}`);
      if (!resolved) {
        return {
          need_user_confirm: {
            reason: 'action not found for preflight (resolveAction failed)',
            details: { skill, action: actionId, node_id: node?.id },
          },
        };
      }

      const req = resolved.action?.requires_queries;
      if (Array.isArray(req) && req.length > 0) {
        const missing = req.filter((q: any) => typeof q === 'string' && ctx?.runtime?.query?.[q] === undefined);
        if (missing.length > 0) {
          return {
            need_user_confirm: {
              reason: 'missing required queries for action',
              details: { node_id: node?.id, action_ref: `${skill}/${actionId}`, missing_queries: missing },
            },
          };
        }
      }
    }

    return await this.inner.execute(node, ctx, options);
  }
}

export class PolicyGateExecutor implements Executor {
  private readonly approvedByActionKey = new Set<string>();

  constructor(
    private readonly sdk: any,
    private readonly inner: Executor,
    private readonly opts: {
      pack?: any;
      yes?: boolean;
    }
  ) {}

  supports(node: any): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(node: any, ctx: any, options?: any): Promise<any> {
    const pack = this.opts.pack;
    const policy = pack?.policy;
    if (!policy || classifyIo(node) !== 'write') return await this.inner.execute(node, ctx, options);

    const skill = node?.source?.skill;
    const actionId = node?.source?.action;
    const workflowNodeId = String(node?.source?.node_id ?? '');
    if (typeof skill !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
      return await this.inner.execute(node, ctx, options);
    }

    const parsed = this.sdk.parseSkillRef ? this.sdk.parseSkillRef(skill) : { protocol: null, version: null };
    const protocol = parsed?.protocol ? String(parsed.protocol) : '';
    const version = parsed?.version ? String(parsed.version) : '';

    const actionKey = protocol ? `${protocol}.${actionId}` : `${skill}/${actionId}`;
    const gateKey = workflowNodeId ? `${workflowNodeId}:${actionKey}` : actionKey;
    if (this.approvedByActionKey.has(gateKey)) return await this.inner.execute(node, ctx, options);

    const resolved = this.sdk.resolveAction(ctx, `${skill}/${actionId}`);
    if (!resolved) {
      return {
        need_user_confirm: {
          reason: 'action not found for policy gate (resolveAction failed)',
          details: { skill, action: actionId, node_id: node?.id },
        },
      };
    }

    const baseRiskLevel = resolved.action?.risk_level;
    const risk_level = typeof baseRiskLevel === 'number' ? baseRiskLevel : 3;

    const tags1 = Array.isArray(resolved.action?.risk_tags) ? resolved.action.risk_tags : [];
    const overrides = pack?.overrides?.actions;
    const override =
      overrides && typeof overrides === 'object'
        ? (overrides[actionKey] ?? (protocol && version ? overrides[`${protocol}@${version}.${actionId}`] : undefined))
        : undefined;
    const tags2 = Array.isArray(override?.risk_tags) ? override.risk_tags : [];
    const risk_tags = uniqStrings([...tags1, ...tags2]);

    const tokenPolicy = pack?.token_policy;
    const res = this.sdk.validateConstraints
      ? this.sdk.validateConstraints(policy, tokenPolicy, { chain: node?.chain, risk_level, risk_tags })
      : { requires_approval: false, approval_reasons: [] };

    if (res?.requires_approval) {
      if (!this.opts.yes) {
        return {
          need_user_confirm: {
            reason: 'policy approval required',
            details: {
              node_id: node?.id,
              workflow_node_id: workflowNodeId || undefined,
              step_id: node?.source?.step_id,
              action_ref: `${skill}/${actionId}`,
              action_key: actionKey,
              risk_level,
              risk_tags,
              approval_reasons: res.approval_reasons ?? [],
              pack: packMeta(pack),
              policy: policyApprovalsSummary(policy),
            },
          },
        };
      }
      this.approvedByActionKey.add(gateKey);
    }

    return await this.inner.execute(node, ctx, options);
  }
}

export class CalculatedFieldsExecutor implements Executor {
  constructor(
    private readonly sdk: any,
    private readonly inner: Executor
  ) {}

  supports(node: any): boolean {
    return this.inner.supports(node);
  }

  async destroy(): Promise<void> {
    await this.inner.destroy?.();
  }

  async execute(node: any, ctx: any, options?: any): Promise<any> {
    const skill = node?.source?.skill;
    const actionId = node?.source?.action;
    if (typeof skill !== 'string' || typeof actionId !== 'string' || actionId.length === 0) {
      return await this.inner.execute(node, ctx, options);
    }

    const resolved = this.sdk.resolveAction(ctx, `${skill}/${actionId}`);
    if (!resolved) {
      return {
        need_user_confirm: {
          reason: 'action not found for calculated_fields (resolveAction failed)',
          details: { skill, action: actionId, node_id: node?.id },
        },
      };
    }

    const calculated = resolved.action?.calculated_fields;
    if (!calculated || typeof calculated !== 'object') {
      return await this.inner.execute(node, ctx, options);
    }

    const order = topoOrderCalculatedFields(calculated);
    const computed: Record<string, unknown> = {};

    for (const name of order) {
      const def = (calculated as any)[name];
      const expr = def?.expr;
      if (!expr) continue;

      try {
        const resolvedParams = options?.resolved_params ?? {};
        const detect = options?.detect;
        const evalOpts =
          detect || resolvedParams
            ? { root_overrides: { params: resolvedParams }, detect }
            : undefined;
        const v = detect
          ? await this.sdk.evaluateValueRefAsync(expr, ctx, evalOpts)
          : this.sdk.evaluateValueRef(expr, ctx, evalOpts);
        computed[name] = v;
      } catch (e) {
        const msg = (e as Error)?.message ?? String(e);
        const needsDetect =
          msg.includes('Detect kind') || msg.includes('Async detect') || msg.includes('Detect provider');
        return {
          need_user_confirm: {
            reason: needsDetect ? 'calculated_fields requires detect resolution' : 'calculated_fields evaluation failed',
            details: {
              node_id: node?.id,
              action_ref: `${skill}/${actionId}`,
              field: name,
              error: msg,
            },
          },
        };
      }
    }

    const patches = [
      { op: 'merge', path: 'calculated', value: computed },
      { op: 'merge', path: `nodes.${String(node?.id ?? '')}.calculated`, value: computed },
    ];
    this.sdk.applyRuntimePatches(ctx, patches);

    return await this.inner.execute(node, ctx, options);
  }
}

function classifyIo(node: any): 'read' | 'write' {
  const t = String(node?.execution?.type ?? '');
  if (t === 'evm_read' || t === 'evm_multiread' || t === 'solana_read') return 'read';
  return 'write';
}

function uniqStrings(xs: string[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const x of xs) {
    if (typeof x !== 'string' || x.length === 0) continue;
    if (seen.has(x)) continue;
    seen.add(x);
    out.push(x);
  }
  return out;
}

function packMeta(pack: any): { name?: string; version?: string } | undefined {
  if (!pack || typeof pack !== 'object') return undefined;
  const meta = pack.meta && typeof pack.meta === 'object' ? pack.meta : null;
  const name = meta?.name ?? pack.name;
  const version = meta?.version ?? pack.version;
  const out: any = {};
  if (name) out.name = String(name);
  if (version) out.version = String(version);
  return Object.keys(out).length > 0 ? out : undefined;
}

function policyApprovalsSummary(policy: any): unknown {
  if (!policy || typeof policy !== 'object') return undefined;
  const approvals = policy.approvals;
  if (!approvals || typeof approvals !== 'object') return undefined;
  const out: any = {};
  if (approvals.auto_execute_max_risk_level !== undefined) out.auto_execute_max_risk_level = approvals.auto_execute_max_risk_level;
  if (approvals.require_approval_min_risk_level !== undefined) out.require_approval_min_risk_level = approvals.require_approval_min_risk_level;
  if (Object.keys(out).length === 0) return undefined;
  return out;
}

function compileWritePreview(sdk: SdkLike, node: any, ctx: any, resolvedParams: Record<string, unknown>): unknown {
  const t = String(node?.execution?.type ?? '');
  try {
    if ((t === 'evm_call' || t === 'evm_multicall') && typeof sdk.compileEvmExecution === 'function') {
      const compiled = sdk.compileEvmExecution(node.execution, ctx, { chain: node.chain, params: resolvedParams });
      return {
        chain: compiled.chain,
        chainId: compiled.chainId,
        to: compiled.to,
        data: compiled.data,
        value: String(compiled.value),
        abi: compiled.abi?.name ?? undefined,
      };
    }
    if (t === 'solana_instruction' && sdk.solana?.compileSolanaInstruction) {
      const compiled = sdk.solana.compileSolanaInstruction(node.execution, ctx, { chain: node.chain, params: resolvedParams });
      const program = compiled.programId?.toBase58 ? compiled.programId.toBase58() : String(compiled.programId ?? '');
      return {
        chain: node.chain,
        program,
        instruction: compiled.instruction,
        lookup_tables: compiled.lookupTables ?? [],
        compute_units: compiled.computeUnits ?? undefined,
      };
    }
  } catch (e) {
    return { chain: node.chain, exec_type: t, compile_error: (e as Error)?.message ?? String(e) };
  }
  return { chain: node.chain, exec_type: t };
}

function isEvmFailureStatus(status: unknown): boolean {
  if (status === 0 || status === false) return true;
  if (status === 1 || status === true) return false;
  if (typeof status === 'string') {
    const s = status.toLowerCase();
    if (s === '0x0' || s === '0') return true;
    if (s === '0x1' || s === '1') return false;
  }
  return false;
}

function topoOrderCalculatedFields(
  calculated: Record<string, { inputs?: string[] }>
): string[] {
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
    // cycle: fall back to stable original order (still deterministic)
    return names.slice().sort((a, b) => (originalIndex.get(a) ?? 0) - (originalIndex.get(b) ?? 0));
  }

  return ordered;
}

function extractCalculatedDep(input: string): string | null {
  // Inputs are like "calculated.amount_in_atomic" or "query.quote.amount"
  if (!input.startsWith('calculated.')) return null;
  const rest = input.slice('calculated.'.length);
  const first = rest.split('.', 1)[0];
  return first && /^[A-Za-z_][A-Za-z0-9_]*$/.test(first) ? first : null;
}
