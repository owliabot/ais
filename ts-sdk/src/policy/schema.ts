import { z } from 'zod';

export const WritePreviewSchema = z
  .object({
    kind: z.string().min(1),
    chain: z.string().min(1).optional(),
    exec_type: z.string().min(1).optional(),
    compile_error: z.string().min(1).optional(),
  })
  .catchall(z.unknown());

export const PolicyGateInputSchema = z
  .object({
    node_id: z.string().min(1).optional(),
    workflow_node_id: z.string().min(1).optional(),
    step_id: z.string().min(1).optional(),
    action_ref: z.string().min(1).optional(),
    action_key: z.string().min(1).optional(),
    chain: z.string().min(1),
    params: z.record(z.unknown()).optional(),
    preview: WritePreviewSchema.optional(),
    hard_block_fields: z.array(z.string().min(1)).optional(),
    missing_fields: z.array(z.string().min(1)).optional(),
    unknown_fields: z.array(z.string().min(1)).optional(),
    field_sources: z.record(z.array(z.string().min(1))).optional(),
    token_address: z.string().min(1).optional(),
    token_symbol: z.string().min(1).optional(),
    spend_amount: z.string().min(1).optional(),
    approval_amount: z.string().min(1).optional(),
    slippage_bps: z.number().int().nonnegative().optional(),
    unlimited_approval: z.boolean().optional(),
    risk_level: z.number().int().min(1).max(5).optional(),
    risk_tags: z.array(z.string()).optional(),
    spender_address: z.string().min(1).optional(),
    owner_address: z.string().min(1).optional(),
    mint_address: z.string().min(1).optional(),
  })
  .strict();

export const PolicyGateOutputSchema = z.discriminatedUnion('kind', [
  z
    .object({
      ok: z.literal(true),
      kind: z.literal('ok'),
      reason: z.string().min(1).optional(),
      details: z.record(z.unknown()).optional(),
    })
    .strict(),
  z
    .object({
      ok: z.literal(false),
      kind: z.literal('need_user_confirm'),
      reason: z.string().min(1),
      details: z.record(z.unknown()).optional(),
    })
    .strict(),
  z
    .object({
      ok: z.literal(false),
      kind: z.literal('hard_block'),
      reason: z.string().min(1),
      details: z.record(z.unknown()).optional(),
    })
    .strict(),
]);

export type PolicyGateInputShape = z.infer<typeof PolicyGateInputSchema>;
export type PolicyGateOutputShape = z.infer<typeof PolicyGateOutputSchema>;

export type PolicyFieldNullSemantics = 'required_missing' | 'allowed_unknown' | 'not_applicable';

export interface PolicyGateFieldDictionaryEntry {
  value_format: string;
  source_priority: string[];
  null_semantics: PolicyFieldNullSemantics;
  required_when?: string;
  audit_purpose: string;
}

export const POLICY_GATE_INPUT_FIELD_DICTIONARY: Record<string, PolicyGateFieldDictionaryEntry> = {
  hard_block_fields: {
    value_format: 'string[]',
    source_priority: ['extractPolicyGateInput classification'],
    null_semantics: 'not_applicable',
    audit_purpose: 'Fields that must be known pre-execution; non-empty means hard block.',
  },
  chain: {
    value_format: 'caip-2 string',
    source_priority: ['node.chain', 'preview.chain', 'runtime.ctx.chain_id'],
    null_semantics: 'required_missing',
    required_when: 'all write nodes',
    audit_purpose: 'Bind policy checks to chain scope.',
  },
  risk_level: {
    value_format: 'integer 1..5',
    source_priority: ['action.meta.risk_level', 'default=3'],
    null_semantics: 'allowed_unknown',
    audit_purpose: 'Drive approval threshold decisions.',
  },
  risk_tags: {
    value_format: 'string[]',
    source_priority: ['action.meta.risk_tags', 'pack.overrides.actions.*.risk_tags'],
    null_semantics: 'allowed_unknown',
    audit_purpose: 'Explain why a gate asks for confirmation.',
  },
  token_address: {
    value_format: 'address/pubkey string',
    source_priority: ['params.token*', 'preview.evm/solana'],
    null_semantics: 'allowed_unknown',
    audit_purpose: 'Token allowlist and asset traceability.',
  },
  spend_amount: {
    value_format: 'base-unit integer string',
    source_priority: [
      'params.spend_amount|amount_in|amount',
      'calculated.spend_amount|amount_in|amount',
      'detect_result.spend_amount|amount_in|amount',
      'preview.function/data',
    ],
    null_semantics: 'required_missing',
    required_when: 'swap/transfer-like write actions',
    audit_purpose: 'Enforce spend limits and user exposure.',
  },
  slippage_bps: {
    value_format: 'integer bps',
    source_priority: [
      'params.slippage_bps|max_slippage_bps',
      'calculated.slippage_bps|max_slippage_bps',
      'detect_result.slippage_bps|max_slippage_bps',
      'preview.args',
    ],
    null_semantics: 'required_missing',
    required_when: 'swap-like write actions',
    audit_purpose: 'Enforce max slippage hard constraints.',
  },
  approval_amount: {
    value_format: 'base-unit integer string',
    source_priority: [
      'params.approval_amount|max_approval',
      'calculated.approval_amount|max_approval',
      'detect_result.approval_amount|max_approval',
      'preview.approve args/data',
    ],
    null_semantics: 'required_missing',
    required_when: 'approve-like write actions',
    audit_purpose: 'Check approval risk and cap violations.',
  },
  unlimited_approval: {
    value_format: 'boolean',
    source_priority: ['params.unlimited_approval', 'inferred from approval_amount/preview'],
    null_semantics: 'allowed_unknown',
    audit_purpose: 'Block unlimited approvals when disallowed.',
  },
  spender_address: {
    value_format: 'address/pubkey string',
    source_priority: ['preview.approve args/accounts'],
    null_semantics: 'required_missing',
    required_when: 'approve-like write actions',
    audit_purpose: 'Attribute approval destination.',
  },
};

export const POLICY_GATE_OUTPUT_FIELD_DICTIONARY: Record<string, PolicyGateFieldDictionaryEntry> = {
  kind: {
    value_format: "'ok'|'need_user_confirm'|'hard_block'",
    source_priority: ['enforcePolicyGate decision'],
    null_semantics: 'required_missing',
    audit_purpose: 'Primary control-flow decision for agent/runner.',
  },
  reason: {
    value_format: 'string',
    source_priority: ['enforcePolicyGate'],
    null_semantics: 'required_missing',
    required_when: 'kind != ok',
    audit_purpose: 'Human-readable gate explanation.',
  },
  details: {
    value_format: 'object',
    source_priority: ['constraint engine + gate input snapshot'],
    null_semantics: 'allowed_unknown',
    audit_purpose: 'Machine-readable evidence for UI/audit.',
  },
};

export function validatePolicyGateInput(input: unknown): { ok: true; value: PolicyGateInputShape } | { ok: false; issues: string[] } {
  const parsed = PolicyGateInputSchema.safeParse(input);
  if (parsed.success) return { ok: true, value: parsed.data };
  return {
    ok: false,
    issues: parsed.error.issues.map((issue) => `${issue.path.join('.') || '<root>'}: ${issue.message}`),
  };
}

export function validatePolicyGateOutput(output: unknown): { ok: true; value: PolicyGateOutputShape } | { ok: false; issues: string[] } {
  const parsed = PolicyGateOutputSchema.safeParse(output);
  if (parsed.success) return { ok: true, value: parsed.data };
  return {
    ok: false,
    issues: parsed.error.issues.map((issue) => `${issue.path.join('.') || '<root>'}: ${issue.message}`),
  };
}
