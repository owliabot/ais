import type { ExecutionPlanNode } from '../execution/plan.js';
import { compileEvmExecution, type CompiledEvmAbiRequest } from '../execution/evm/compiler.js';
import { compileSolanaInstruction } from '../execution/solana/compiler.js';
import type { ResolverContext } from '../resolver/context.js';
import { parseProtocolRef } from '../resolver/reference.js';
import { isCoreExecutionType, type Pack, type SolanaInstruction } from '../schema/index.js';
import type { ConstraintInput } from '../validator/constraint.js';
import { validateConstraints } from '../validator/constraint.js';

export type EnforcementKind = 'ok' | 'need_user_confirm' | 'hard_block';

export interface EnforcementResult {
  ok: boolean;
  kind: EnforcementKind;
  reason?: string;
  details?: Record<string, unknown>;
}

export interface DetectAllowInput {
  kind: string;
  provider?: string;
  chain?: string;
  strict_allowlist?: boolean;
}

export interface DetectProviderPickInput {
  kind: string;
  provider?: string;
  chain?: string;
  candidates?: string[];
  strict_allowlist?: boolean;
}

export interface DetectProviderPickResult extends EnforcementResult {
  provider?: string;
}

export interface ExecutionPluginAllowInput {
  type: string;
  chain?: string;
  strict_allowlist?: boolean;
}

export interface WritePreview {
  kind: string;
  chain?: string;
  exec_type?: string;
  compile_error?: string;
  [key: string]: unknown;
}

export interface CompileWritePreviewOptions {
  node: ExecutionPlanNode;
  ctx: ResolverContext;
  resolved_params?: Record<string, unknown>;
}

export interface PolicyGateInput extends ConstraintInput {
  node_id?: string;
  workflow_node_id?: string;
  step_id?: string;
  action_ref?: string;
  action_key?: string;
  chain: string;
  params?: Record<string, unknown>;
  preview?: WritePreview;
  hard_block_fields?: string[];
  missing_fields?: string[];
  unknown_fields?: string[];
  field_sources?: Record<string, string[]>;
  spender_address?: string;
  owner_address?: string;
  mint_address?: string;
}

export interface ExtractPolicyGateInputOptions {
  node: ExecutionPlanNode;
  ctx: ResolverContext;
  pack?: Pack;
  resolved_params?: Record<string, unknown>;
  action_risk_level?: number;
  action_risk_tags?: string[];
  runtime_risk_level?: number;
  runtime_risk_tags?: string[];
  detect_result?: Record<string, unknown>;
  preview?: WritePreview;
}

export interface EnforcePolicyGateOptions {
  strict_allowlist?: boolean;
}

const OK: EnforcementResult = { ok: true, kind: 'ok' };

export function checkDetectAllowed(pack: Pack | undefined, input: DetectAllowInput): EnforcementResult {
  const strictAllowlist = input.strict_allowlist ?? false;
  const enabled = pack?.providers?.detect?.enabled ?? [];
  if (enabled.length === 0) {
    if (strictAllowlist) {
      return {
        ok: false,
        kind: 'hard_block',
        reason: 'detect allowlist is empty',
        details: {
          kind: input.kind,
          provider: input.provider,
          chain: input.chain,
          pack_enabled: [],
          pack_meta: summarizePackMeta(pack),
        },
      };
    }
    return OK;
  }

  const candidates = enabled.filter((entry) => entry.kind === input.kind && chainAllowed(entry.chains, input.chain));
  if (candidates.length === 0) {
    return {
      ok: false,
      kind: 'hard_block',
      reason: 'detect kind is not allowlisted by pack',
      details: {
        kind: input.kind,
        provider: input.provider,
        chain: input.chain,
        pack_enabled: enabled.map((entry) => ({ kind: entry.kind, provider: entry.provider, chains: entry.chains })),
        pack_meta: summarizePackMeta(pack),
      },
    };
  }

  if (input.provider) {
    const match = candidates.some((entry) => entry.provider === input.provider);
    if (!match) {
      return {
        ok: false,
        kind: 'hard_block',
        reason: 'detect provider is not allowlisted by pack',
        details: {
          kind: input.kind,
          provider: input.provider,
          chain: input.chain,
          pack_enabled_for_kind: candidates.map((entry) => ({
            provider: entry.provider,
            priority: entry.priority,
            chains: entry.chains,
          })),
          pack_meta: summarizePackMeta(pack),
        },
      };
    }
  }

  return OK;
}

export function pickDetectProvider(pack: Pack | undefined, input: DetectProviderPickInput): DetectProviderPickResult {
  const check = checkDetectAllowed(pack, input);
  if (!check.ok) return { ...check, provider: undefined };

  if (input.provider) return { ok: true, kind: 'ok', provider: input.provider };

  const enabled = pack?.providers?.detect?.enabled ?? [];
  const strictAllowlist = input.strict_allowlist ?? false;
  if (enabled.length === 0) {
    if (strictAllowlist) {
      return {
        ok: false,
        kind: 'hard_block',
        reason: 'detect allowlist is empty',
        provider: undefined,
        details: {
          kind: input.kind,
          chain: input.chain,
          candidates: input.candidates,
          pack_enabled: [],
          pack_meta: summarizePackMeta(pack),
        },
      };
    }
    return { ok: true, kind: 'ok', provider: undefined };
  }

  const matches = enabled
    .filter((entry) => entry.kind === input.kind && chainAllowed(entry.chains, input.chain))
    .slice()
    .sort((left, right) => {
      const lp = typeof left.priority === 'number' ? left.priority : 0;
      const rp = typeof right.priority === 'number' ? right.priority : 0;
      return rp - lp || left.provider.localeCompare(right.provider);
    });
  const byCandidates = applyProviderCandidates(matches, input.candidates);
  if (byCandidates.length === 0) {
    return {
      ok: false,
      kind: 'hard_block',
      reason: 'no detect provider available for kind/chain in pack allowlist',
      provider: undefined,
      details: {
        kind: input.kind,
        chain: input.chain,
        candidates: input.candidates,
        pack_enabled_for_kind: enabled
          .filter((entry) => entry.kind === input.kind)
          .map((entry) => ({
            provider: entry.provider,
            priority: entry.priority,
            chains: entry.chains,
          })),
        eligible_for_chain: matches.map((entry) => ({
          provider: entry.provider,
          priority: entry.priority,
          chains: entry.chains,
        })),
        pack_meta: summarizePackMeta(pack),
      },
    };
  }
  return { ok: true, kind: 'ok', provider: byCandidates[0]!.provider };
}

export function checkExecutionPluginAllowed(pack: Pack | undefined, input: ExecutionPluginAllowInput): EnforcementResult {
  if (isCoreExecutionType(input.type)) return OK;

  const strictAllowlist = input.strict_allowlist ?? false;
  const enabled = pack?.plugins?.execution?.enabled ?? [];
  if (enabled.length === 0) {
    if (strictAllowlist) {
      return {
        ok: false,
        kind: 'hard_block',
        reason: 'plugin execution allowlist is empty',
        details: { type: input.type, chain: input.chain },
      };
    }
    return OK;
  }

  const allowed = enabled.some((entry) => entry.type === input.type && chainAllowed(entry.chains, input.chain));
  if (!allowed) {
    return {
      ok: false,
      kind: 'hard_block',
      reason: 'plugin execution type is not allowlisted by pack',
      details: {
        type: input.type,
        chain: input.chain,
        allowed: enabled.map((entry) => ({ type: entry.type, chains: entry.chains })),
        pack_meta: summarizePackMeta(pack),
      },
    };
  }
  return OK;
}

export function compileWritePreview(options: CompileWritePreviewOptions): WritePreview {
  const { node, ctx } = options;
  const resolvedParams = options.resolved_params ?? {};
  const execType = node.execution.type;

  try {
    if (execType === 'evm_call') {
      const compiled = compileEvmExecution(node.execution, ctx, {
        chain: node.chain,
        params: resolvedParams,
      }) as CompiledEvmAbiRequest;
      return {
        kind: 'evm_tx',
        chain: compiled.chain,
        chain_id: compiled.chainId,
        exec_type: execType,
        to: compiled.to,
        data: compiled.data,
        value: String(compiled.value),
        function_name: compiled.abi?.name,
        args: sanitizeJsonLike(compiled.args ?? {}),
      };
    }

    if (execType === 'solana_instruction' && isSolanaInstructionExecution(node.execution)) {
      const compiled = compileSolanaInstruction(node.execution, ctx, {
        chain: node.chain,
        params: resolvedParams,
      });
      const accountSummary = Array.isArray(compiled.accounts)
        ? compiled.accounts.map((account) => ({
            name: account.name,
            pubkey: account.pubkey,
            signer: account.isSigner,
            writable: account.isWritable,
          }))
        : compiled.tx.keys.map((key, index) => ({
            index,
            pubkey: key.pubkey.toBase58(),
            signer: key.isSigner,
            writable: key.isWritable,
          }));

      return {
        kind: 'solana_instruction',
        chain: compiled.chain,
        exec_type: execType,
        program_id: compiled.programId.toBase58(),
        instruction: compiled.instruction,
        accounts: accountSummary,
        compute_units: compiled.computeUnits,
        lookup_tables: compiled.lookupTables ?? [],
        data_fields: extractRelevantDataFields(compiled.dataSummary),
        discriminator_summary: summarizeValue(compiled.discriminatorSummary),
        data_summary: summarizeValue(compiled.dataSummary),
      };
    }
  } catch (error) {
    return {
      kind: 'execution',
      chain: node.chain,
      exec_type: execType,
      compile_error: (error as Error)?.message ?? String(error),
    };
  }

  return {
    kind: 'execution',
    chain: node.chain,
    exec_type: execType,
  };
}

export function extractPolicyGateInput(options: ExtractPolicyGateInputOptions): PolicyGateInput {
  const { node, ctx, pack } = options;
  const resolvedParams = options.resolved_params ?? {};
  const protocolRef = asString(node.source?.protocol);
  const actionId = asString(node.source?.action);
  const workflowNodeId = asString(node.source?.node_id) ?? node.id;
  const stepId = asString(node.source?.step_id);
  const preview = options.preview ?? compileWritePreview({ node, ctx, resolved_params: resolvedParams });

  const riskTagsFromAction = Array.isArray(options.action_risk_tags) ? options.action_risk_tags : [];
  const riskTagsFromRuntime = Array.isArray(options.runtime_risk_tags) ? options.runtime_risk_tags : [];
  const overrideTags = collectOverrideRiskTags(pack, protocolRef, actionId);
  const riskTags = uniqStrings([...riskTagsFromAction, ...overrideTags, ...riskTagsFromRuntime]);
  const riskLevel = resolveRiskLevel({
    action_risk_level: options.action_risk_level,
    pack_override_risk_level: collectOverrideRiskLevel(pack, protocolRef, actionId),
    runtime_risk_level: options.runtime_risk_level,
  });
  const riskLevelSource = riskLevel.source;

  const gateInput: PolicyGateInput = {
    node_id: node.id,
    workflow_node_id: workflowNodeId,
    step_id: stepId,
    action_ref: protocolRef && actionId ? `${protocolRef}/${actionId}` : undefined,
    action_key: buildActionKey(protocolRef, actionId),
    chain: node.chain,
    params: resolvedParams,
    preview,
    risk_level: riskLevel.value,
    risk_tags: riskTags,
    field_sources: {},
  };
  setFieldSource(gateInput, 'risk_level', riskLevelSource);
  if (riskTags.length > 0) {
    if (riskTagsFromAction.length > 0) setFieldSource(gateInput, 'risk_tags', 'action');
    if (overrideTags.length > 0) setFieldSource(gateInput, 'risk_tags', 'pack_override');
    if (riskTagsFromRuntime.length > 0) setFieldSource(gateInput, 'risk_tags', 'runtime');
  }

  extractFromResolvedParams(gateInput, resolvedParams, ctx, options.detect_result);
  extractFromPreview(gateInput, preview);

  const runtimeChain = asString((ctx.runtime.ctx as Record<string, unknown> | undefined)?.chain_id);
  if (runtimeChain && !gateInput.chain) setGateField(gateInput, 'chain', runtimeChain, 'runtime');

  gateInput.hard_block_fields = collectHardBlockFields(gateInput, preview);
  if (gateInput.hard_block_fields.length === 0) delete gateInput.hard_block_fields;
  gateInput.missing_fields = collectMissingFields(gateInput, preview);
  if (gateInput.missing_fields.length === 0) delete gateInput.missing_fields;
  gateInput.unknown_fields = collectUnknownFields(gateInput, preview);
  if (gateInput.unknown_fields.length === 0) delete gateInput.unknown_fields;

  return gateInput;
}

export function enforcePolicyGate(
  pack: Pack | undefined,
  gateInput: PolicyGateInput,
  _options: EnforcePolicyGateOptions = {}
): EnforcementResult {
  const policy = pack?.policy;
  if (!policy) return OK;

  if (Array.isArray(gateInput.hard_block_fields) && gateInput.hard_block_fields.length > 0) {
    return {
      ok: false,
      kind: 'hard_block',
      reason: 'policy gate required fields are missing',
      details: {
        gate_input: gateInput,
        hard_block_fields: gateInput.hard_block_fields,
      },
    };
  }

  if (Array.isArray(gateInput.missing_fields) && gateInput.missing_fields.length > 0) {
    return {
      ok: false,
      kind: 'need_user_confirm',
      reason: 'policy gate input is incomplete',
      details: {
        gate_input: gateInput,
        missing_fields: gateInput.missing_fields,
      },
    };
  }
  if (Array.isArray(gateInput.unknown_fields) && gateInput.unknown_fields.length > 0) {
    return {
      ok: false,
      kind: 'need_user_confirm',
      reason: 'policy gate input has unknown fields',
      details: {
        gate_input: gateInput,
        unknown_fields: gateInput.unknown_fields,
      },
    };
  }

  const check = validateConstraints(policy, pack?.token_policy, gateInput);
  if (!check.valid) {
    return {
      ok: false,
      kind: 'hard_block',
      reason: 'policy hard constraint violation',
      details: {
        gate_input: gateInput,
        violations: check.violations,
      },
    };
  }
  if (check.requires_approval) {
    return {
      ok: false,
      kind: 'need_user_confirm',
      reason: 'policy approval required',
      details: {
        gate_input: gateInput,
        approval_reasons: check.approval_reasons,
      },
    };
  }
  return OK;
}

export function explainPolicyGateResult(result: EnforcementResult): Record<string, unknown> {
  if (result.ok) return { status: 'ok' };
  return {
    status: result.kind,
    reason: result.reason,
    details: result.details,
  };
}

function extractFromResolvedParams(
  gateInput: PolicyGateInput,
  params: Record<string, unknown>,
  ctx: ResolverContext,
  detectResultInput: Record<string, unknown> | undefined
): void {
  const sources = buildExtractionSources(params, ctx, detectResultInput);

  const slippage = parseIntLike(
    readPrioritized(sources, [
      'slippage_bps',
      'max_slippage_bps',
    ])
  );
  if (slippage !== undefined) setGateField(gateInput, 'slippage_bps', slippage, slippageSource(sources), true);

  const approvalAmount = normalizeAmount(
    readPrioritized(sources, ['approval_amount', 'max_approval'])
  );
  if (approvalAmount) setGateField(gateInput, 'approval_amount', approvalAmount, amountSource('approval_amount', sources), true);

  const spendAmount = normalizeAmount(
    readPrioritized(sources, ['spend_amount', 'amount_in', 'amount'])
  );
  if (spendAmount) setGateField(gateInput, 'spend_amount', spendAmount, amountSource('spend_amount', sources), true);

  const unlimitedApproval = inferUnlimitedApprovalWithSources(sources);
  if (unlimitedApproval !== undefined) setGateField(gateInput, 'unlimited_approval', unlimitedApproval, amountSource('approval_amount', sources), true);

  const token = findTokenCandidate(readPrioritizedSourceRecord(sources));
  if (token) {
    if (typeof token.address === 'string') setGateField(gateInput, 'token_address', token.address, 'params');
    if (typeof token.symbol === 'string') setGateField(gateInput, 'token_symbol', token.symbol, 'params');
    if (typeof token.chain === 'string') setGateField(gateInput, 'chain', token.chain, 'params');
  }
}

function extractFromPreview(gateInput: PolicyGateInput, preview: WritePreview): void {
  if (!preview || typeof preview !== 'object') return;

  if (typeof preview.chain === 'string' && preview.chain.length > 0) {
    setGateField(gateInput, 'chain', preview.chain, 'preview');
  }

  if (preview.kind === 'evm_tx') {
    applyEvmPreview(gateInput, preview);
    return;
  }

  if (preview.kind === 'solana_instruction') {
    applySolanaPreview(gateInput, preview);
  }
}

function applyEvmPreview(gateInput: PolicyGateInput, preview: WritePreview): void {
  const functionName = asString(preview.function_name)?.toLowerCase() ?? '';
  const args = toRecord(preview.args);
  const to = asString(preview.to);

  if (looksLikeApprovalFunction(functionName)) {
    if (!gateInput.token_address && to) setGateField(gateInput, 'token_address', to, 'preview.evm');
    if (!gateInput.spender_address) {
      const spender = asString(readFirst(args, ['spender', '_spender', 'delegate', 'guy']));
      if (spender) setGateField(gateInput, 'spender_address', spender, 'preview.evm');
    }

    if (!gateInput.approval_amount) {
      const approvalAmount = normalizeAmount(readFirst(args, ['amount', 'value', '_value', 'wad']));
      if (approvalAmount) setGateField(gateInput, 'approval_amount', approvalAmount, 'preview.evm');
    }

    if (gateInput.unlimited_approval === undefined) {
      const amountCandidate = readFirst(args, ['amount', 'value', '_value', 'wad']);
      setGateField(gateInput, 'unlimited_approval', looksUnlimited(amountCandidate), 'preview.evm');
    }
  }

  if (!gateInput.spend_amount && looksLikeSwapFunction(functionName)) {
    const amountIn = normalizeAmount(readFirst(args, ['amountIn', 'amount_in', 'amountInMaximum', 'amountInMax']));
    if (amountIn) setGateField(gateInput, 'spend_amount', amountIn, 'preview.evm');
  }

  if (gateInput.slippage_bps === undefined) {
    const bps = parseIntLike(readFirst(args, ['slippageBps', 'maxSlippageBps', 'slippage_bps']));
    if (bps !== undefined) setGateField(gateInput, 'slippage_bps', bps, 'preview.evm');
  }
}

function applySolanaPreview(gateInput: PolicyGateInput, preview: WritePreview): void {
  const instruction = asString(preview.instruction)?.toLowerCase() ?? '';
  const dataSummary = toRecord(preview.data_fields);
  const accounts = asArrayOfRecords(preview.accounts);

  const owner = findAccountPubkey(accounts, ['owner', 'authority']);
  const source = findAccountPubkey(accounts, ['source']);
  const mint = findAccountPubkey(accounts, ['mint']);

  if (!gateInput.owner_address && owner) setGateField(gateInput, 'owner_address', owner, 'preview.solana');
  if (!gateInput.token_address && mint) setGateField(gateInput, 'token_address', mint, 'preview.solana');
  if (!gateInput.mint_address && mint) setGateField(gateInput, 'mint_address', mint, 'preview.solana');

  const amountFromData = normalizeAmount(readFirst(dataSummary, ['amount', 'value']));

  if (instruction === 'approve') {
    if (!gateInput.approval_amount && amountFromData) setGateField(gateInput, 'approval_amount', amountFromData, 'preview.solana');
    if (!gateInput.spender_address) {
      const delegate = findAccountPubkey(accounts, ['delegate']);
      if (delegate) setGateField(gateInput, 'spender_address', delegate, 'preview.solana');
    }
    if (gateInput.unlimited_approval === undefined) {
      setGateField(
        gateInput,
        'unlimited_approval',
        looksUnlimited(readFirst(dataSummary, ['amount', 'value'])),
        'preview.solana'
      );
    }
    return;
  }

  if ((instruction === 'transfer' || instruction === 'transfer_checked') && !gateInput.spend_amount && amountFromData) {
    setGateField(gateInput, 'spend_amount', amountFromData, 'preview.solana');
  }

  if (!gateInput.token_address && source) {
    setGateField(gateInput, 'token_address', source, 'preview.solana');
  }
}

function collectMissingFields(gateInput: PolicyGateInput, preview: WritePreview): string[] {
  const missing: string[] = [];

  if (preview.kind === 'evm_tx') {
    const functionName = asString(preview.function_name)?.toLowerCase() ?? '';
    if (looksLikeApprovalFunction(functionName)) {
      if (!gateInput.token_address) missing.push('token_address');
      if (!gateInput.approval_amount) missing.push('approval_amount');
      if (!gateInput.spender_address) missing.push('spender_address');
    }

    if (looksLikeSwapFunction(functionName)) {
      if (!gateInput.spend_amount) missing.push('spend_amount');
      if (gateInput.slippage_bps === undefined) missing.push('slippage_bps');
    }
  }

  if (preview.kind === 'solana_instruction') {
    const instruction = asString(preview.instruction)?.toLowerCase() ?? '';
    if (instruction === 'approve') {
      if (!gateInput.approval_amount) missing.push('approval_amount');
      if (!gateInput.spender_address) missing.push('spender_address');
      if (!gateInput.token_address && !gateInput.mint_address) missing.push('token_or_mint');
    }
    if (instruction === 'transfer' || instruction === 'transfer_checked') {
      if (!gateInput.spend_amount) missing.push('spend_amount');
      if (!gateInput.token_address && !gateInput.mint_address) missing.push('token_or_mint');
    }
  }

  return uniqStrings(missing);
}

function collectHardBlockFields(gateInput: PolicyGateInput, preview: WritePreview): string[] {
  const required: string[] = [];
  if (preview.kind === 'execution' && typeof preview.compile_error === 'string' && preview.compile_error.length > 0) {
    required.push('preview_compile');
  }
  return uniqStrings(required);
}

function collectUnknownFields(gateInput: PolicyGateInput, preview: WritePreview): string[] {
  const unknown: string[] = [];
  const hasTokenIdentity = Boolean(gateInput.token_address || gateInput.token_symbol || gateInput.mint_address);
  if (!hasTokenIdentity) unknown.push('token_identity');

  if (preview.kind === 'execution' && typeof preview.compile_error === 'string' && preview.compile_error.length > 0) {
    unknown.push('preview_compile');
  }

  if (preview.kind === 'evm_tx') {
    const functionName = asString(preview.function_name)?.toLowerCase() ?? '';
    if (looksLikeSwapFunction(functionName) && !gateInput.spend_amount) unknown.push('spend_amount');
  }

  if (preview.kind === 'solana_instruction') {
    const instruction = asString(preview.instruction)?.toLowerCase() ?? '';
    if ((instruction === 'transfer' || instruction === 'transfer_checked') && !gateInput.spend_amount) {
      unknown.push('spend_amount');
    }
  }

  return uniqStrings(unknown);
}

function chainAllowed(chains: string[] | undefined, chain: string | undefined): boolean {
  if (!Array.isArray(chains) || chains.length === 0) return true;
  if (!chain) return false;
  return chains.includes(chain);
}

function applyProviderCandidates<
  T extends { provider: string }
>(providers: T[], candidates: string[] | undefined): T[] {
  if (!Array.isArray(candidates) || candidates.length === 0) return providers;
  const wanted = new Set(candidates.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0));
  if (wanted.size === 0) return providers;
  return providers.filter((entry) => wanted.has(entry.provider));
}

function summarizePackMeta(pack: Pack | undefined): { name?: string; version?: string } | undefined {
  if (!pack) return undefined;
  const name = asString((pack.meta as Record<string, unknown> | undefined)?.name) ?? asString((pack as Record<string, unknown>).name);
  const version =
    asString((pack.meta as Record<string, unknown> | undefined)?.version) ?? asString((pack as Record<string, unknown>).version);
  if (!name && !version) return undefined;
  return { ...(name ? { name } : {}), ...(version ? { version } : {}) };
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function parseIntLike(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return Math.trunc(value);
  if (typeof value === 'string' && /^-?\d+$/.test(value)) return Number(value);
  if (typeof value === 'bigint') return Number(value);
  return undefined;
}

function resolveRiskLevel(input: {
  action_risk_level?: number;
  pack_override_risk_level?: number;
  runtime_risk_level?: number;
}): { value: number; source: string } {
  if (typeof input.runtime_risk_level === 'number' && Number.isFinite(input.runtime_risk_level)) {
    return { value: clampRiskLevel(input.runtime_risk_level), source: 'runtime' };
  }
  if (typeof input.pack_override_risk_level === 'number' && Number.isFinite(input.pack_override_risk_level)) {
    return { value: clampRiskLevel(input.pack_override_risk_level), source: 'pack_override' };
  }
  if (typeof input.action_risk_level === 'number' && Number.isFinite(input.action_risk_level)) {
    return { value: clampRiskLevel(input.action_risk_level), source: 'action' };
  }
  return { value: 3, source: 'default' };
}

function clampRiskLevel(value: number): number {
  return Math.max(1, Math.min(5, Math.trunc(value)));
}

type ExtractionSourceName = 'params' | 'calculated' | 'detect_result';
type ExtractionSourceMap = Record<ExtractionSourceName, Record<string, unknown>>;

function buildExtractionSources(
  params: Record<string, unknown>,
  ctx: ResolverContext,
  detectResultInput: Record<string, unknown> | undefined
): ExtractionSourceMap {
  const runtime = toRecord((ctx as { runtime?: unknown }).runtime) ?? {};
  const runtimeCalculated = toRecord(runtime.calculated);
  const runtimeCtx = toRecord(runtime.ctx);
  const runtimeDetect = toRecord(runtimeCtx?.detect_result);
  return {
    params,
    calculated: resolveSourceRecord(params, 'calculated') ?? runtimeCalculated ?? {},
    detect_result: detectResultInput ?? resolveSourceRecord(params, 'detect_result') ?? runtimeDetect ?? {},
  };
}

function resolveSourceRecord(
  root: Record<string, unknown>,
  key: string
): Record<string, unknown> | null {
  return toRecord(root[key]) ?? null;
}

function readPrioritized(sources: ExtractionSourceMap, keys: string[]): unknown {
  const ordered: ExtractionSourceName[] = ['params', 'calculated', 'detect_result'];
  for (const source of ordered) {
    const value = readFirst(sources[source], keys);
    if (value !== undefined) return value;
  }
  return undefined;
}

function readPrioritizedSourceRecord(sources: ExtractionSourceMap): Record<string, unknown> {
  return { ...sources.detect_result, ...sources.calculated, ...sources.params };
}

function sourceOfFirst(sources: ExtractionSourceMap, keys: string[]): ExtractionSourceName | null {
  const ordered: ExtractionSourceName[] = ['params', 'calculated', 'detect_result'];
  for (const source of ordered) {
    const value = readFirst(sources[source], keys);
    if (value !== undefined) return source;
  }
  return null;
}

function slippageSource(sources: ExtractionSourceMap): string {
  return sourceOfFirst(sources, ['slippage_bps', 'max_slippage_bps']) ?? 'params';
}

function amountSource(_field: 'approval_amount' | 'spend_amount', sources: ExtractionSourceMap): string {
  const source = sourceOfFirst(sources, ['approval_amount', 'max_approval', 'spend_amount', 'amount_in', 'amount']);
  return source ?? 'params';
}

function readFirst(obj: Record<string, unknown>, keys: string[]): unknown {
  for (const key of keys) {
    if (obj[key] !== undefined) return obj[key];
  }
  return undefined;
}

function inferUnlimitedApproval(params: Record<string, unknown>): boolean | undefined {
  return inferUnlimitedApprovalWithSources({
    params,
    calculated: {},
    detect_result: {},
  });
}

function inferUnlimitedApprovalWithSources(sources: ExtractionSourceMap): boolean | undefined {
  const direct = readPrioritized(sources, ['unlimited_approval']);
  if (typeof direct === 'boolean') return direct;
  const approvalAmount = readPrioritized(sources, ['approval_amount', 'max_approval', 'amount']);
  if (typeof approvalAmount === 'string' && approvalAmount.toLowerCase() === 'max') return true;
  if (looksUnlimited(approvalAmount)) return true;
  return undefined;
}

function looksUnlimited(value: unknown): boolean {
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    if (normalized === 'max') return true;
    if (/^0x[f]+$/i.test(normalized)) return true;
    if (/^\d+$/.test(normalized)) {
      try {
        return BigInt(normalized) >= MAX_UINT256;
      } catch {
        return false;
      }
    }
  }
  if (typeof value === 'bigint') return value >= MAX_UINT256;
  return false;
}

function normalizeAmount(value: unknown): string | undefined {
  if (typeof value === 'bigint') return value.toString();
  if (typeof value === 'number' && Number.isFinite(value)) return Math.trunc(value).toString();
  if (typeof value !== 'string') return undefined;

  const trimmed = value.trim();
  if (!trimmed) return undefined;

  if (/^-?\d+$/.test(trimmed)) return trimmed;
  if (/^0x[0-9a-f]+$/i.test(trimmed)) {
    try {
      return BigInt(trimmed).toString();
    } catch {
      return undefined;
    }
  }
  return trimmed;
}

function findTokenCandidate(params: Record<string, unknown>): { chain?: string; address?: string; symbol?: string } | null {
  const keys = ['token', 'token_in', 'asset', 'token_out'];
  for (const key of keys) {
    const token = params[key];
    if (!token || typeof token !== 'object' || Array.isArray(token)) continue;
    const rec = token as Record<string, unknown>;
    const address = asString(rec.address);
    const symbol = asString(rec.symbol);
    const chain = asString(rec.chain_id) ?? asString(rec.chain);
    if (address || symbol || chain) {
      return { chain, address, symbol };
    }
  }
  return null;
}

function uniqStrings(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

function collectOverrideRiskTags(pack: Pack | undefined, protocolRef: string | undefined, actionId: string | undefined): string[] {
  if (!pack || !protocolRef || !actionId) return [];
  const overrides = pack.overrides?.actions;
  if (!overrides) return [];
  const parsed = parseProtocolRef(protocolRef);
  const key1 = `${parsed.protocol}.${actionId}`;
  const key2 = parsed.version ? `${parsed.protocol}@${parsed.version}.${actionId}` : '';
  const override = overrides[key1] ?? (key2 ? overrides[key2] : undefined);
  return Array.isArray(override?.risk_tags) ? override.risk_tags.filter((t): t is string => typeof t === 'string') : [];
}

function collectOverrideRiskLevel(pack: Pack | undefined, protocolRef: string | undefined, actionId: string | undefined): number | undefined {
  if (!pack || !protocolRef || !actionId) return undefined;
  const overrides = pack.overrides?.actions;
  if (!overrides) return undefined;
  const parsed = parseProtocolRef(protocolRef);
  const key1 = `${parsed.protocol}.${actionId}`;
  const key2 = parsed.version ? `${parsed.protocol}@${parsed.version}.${actionId}` : '';
  const override = overrides[key1] ?? (key2 ? overrides[key2] : undefined);
  return typeof override?.risk_level === 'number' && Number.isFinite(override.risk_level)
    ? clampRiskLevel(override.risk_level)
    : undefined;
}

function buildActionKey(protocolRef: string | undefined, actionId: string | undefined): string | undefined {
  if (!protocolRef || !actionId) return undefined;
  const parsed = parseProtocolRef(protocolRef);
  return `${parsed.protocol}.${actionId}`;
}

function looksLikeApprovalFunction(functionName: string): boolean {
  return functionName === 'approve' || functionName.endsWith('approve');
}

function looksLikeSwapFunction(functionName: string): boolean {
  return functionName.includes('swap') || functionName.includes('exactinput') || functionName.includes('exactoutput');
}

function toRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
}

function asArrayOfRecords(value: unknown): Record<string, unknown>[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is Record<string, unknown> => !!entry && typeof entry === 'object' && !Array.isArray(entry));
}

function findAccountPubkey(accounts: Record<string, unknown>[], names: string[]): string | undefined {
  for (const account of accounts) {
    const accountName = asString(account.name)?.toLowerCase();
    if (!accountName) continue;
    if (!names.includes(accountName)) continue;
    const pubkey = asString(account.pubkey);
    if (pubkey) return pubkey;
  }
  return undefined;
}

function sanitizeJsonLike(value: unknown): unknown {
  if (typeof value === 'bigint') return value.toString();
  if (Array.isArray(value)) return value.map((entry) => sanitizeJsonLike(entry));
  if (!value || typeof value !== 'object') return value;

  const input = value as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const [key, entry] of Object.entries(input)) {
    out[key] = sanitizeJsonLike(entry);
  }
  return out;
}

function summarizeValue(value: unknown): unknown {
  if (value === null || value === undefined) return value;
  if (typeof value === 'string') return value.length <= 80 ? value : `${value.slice(0, 77)}...`;
  if (typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'bigint') return value.toString();
  if (Array.isArray(value)) {
    return {
      type: 'array',
      length: value.length,
    };
  }
  if (typeof value === 'object') {
    const keys = Object.keys(value as Record<string, unknown>);
    return {
      type: 'object',
      keys: keys.slice(0, 8),
    };
  }
  return String(value);
}

const MAX_UINT256 = (1n << 256n) - 1n;

function extractRelevantDataFields(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const source = value as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of ['amount', 'value', 'decimals']) {
    if (source[key] !== undefined) out[key] = source[key];
  }
  return out;
}

function isSolanaInstructionExecution(execution: ExecutionPlanNode['execution']): execution is SolanaInstruction {
  return execution.type === 'solana_instruction';
}

function setGateField(
  gateInput: PolicyGateInput,
  field: keyof PolicyGateInput,
  value: unknown,
  source: string,
  overwrite = false
): void {
  if (value === undefined || value === null) return;
  const record = gateInput as unknown as Record<string, unknown>;
  const current = record[field];
  if (current !== undefined && !overwrite) return;
  record[field] = value;
  setFieldSource(gateInput, String(field), source);
}

function setFieldSource(gateInput: PolicyGateInput, field: string, source: string): void {
  const sources = gateInput.field_sources ?? {};
  const existing = Array.isArray(sources[field]) ? sources[field]! : [];
  if (!existing.includes(source)) sources[field] = [...existing, source];
  gateInput.field_sources = sources;
}
