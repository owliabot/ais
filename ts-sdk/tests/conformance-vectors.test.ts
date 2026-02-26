import { describe, it, expect } from 'vitest';
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  canonicalizeJcs,
  specHashKeccak256,
  evaluateCEL,
  encodeJsonAbiFunctionCall,
  selectExecutionSpec,
  parseProtocolSpec,
  parseWorkflow,
  createContext,
  registerProtocol,
  buildWorkflowExecutionPlan,
} from '../src/index.js';

type VectorFile = {
  schema: 'ais-conformance/0.0.2';
  profile?: 'core' | 'extended' | 'mixed';
  cases: VectorCase[];
};

type ProfileManifest = {
  schema: 'ais-conformance-profile/0.0.1';
  profile: 'core' | 'extended';
  vector_files: string[];
};

type VectorCase =
  | {
      id: string;
      kind: 'jcs_canonicalize';
      input: { value: unknown };
      expect: { canonical: string; specHashKeccak256: string };
    }
  | {
      id: string;
      kind: 'cel_eval';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { value_bigint: string };
    }
  | {
      id: string;
      kind: 'cel_eval_string';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { value_string: string };
    }
  | {
      id: string;
      kind: 'cel_eval_error';
      input: { expression: string; context?: Record<string, unknown> };
      expect: { message_includes?: string };
    }
  | {
      id: string;
      kind: 'evm_json_abi_encode';
      input: { abi: any; args: Record<string, unknown> };
      expect: { data: string };
    }
  | {
      id: string;
      kind: 'select_execution_spec';
      input: { chain: string; execution: Record<string, any> };
      expect: { type: string };
    }
  | {
      id: string;
      kind: 'select_execution_spec_error';
      input: { chain: string; execution: Record<string, any> };
      expect: { message_includes?: string };
    }
  | {
      id: string;
      kind: 'workflow_plan';
      input: { protocols_yaml: string[]; workflow_yaml: string; golden_file: string };
    }
  | {
      id: string;
      kind: 'confirmation_hash';
      input: { summary: Record<string, unknown> };
      expect: { hash_sha256_hex: string };
    }
  | {
      id: string;
      kind: 'json_schema_validate';
      input: { schema_id: string; value: unknown };
      expect: { valid: boolean };
    }
  | {
      id: string;
      kind: 'pack_approvals_decision';
      input: {
        approvals: {
          mode?: 'safe' | 'assist' | 'yolo';
          auto_execute_max_risk_level?: number;
          require_approval_min_risk_level?: number;
          llm_may_approve_max_risk_level?: number;
        };
        risk_level: number;
      };
      expect: { needs_confirmation: boolean; confirmer: 'none' | 'human' | 'llm' | 'auto' };
    }
  | {
      id: string;
      kind: 'policy_gate_missingness_decision';
      input: {
        hard_block_fields: string[];
        missing_fields: string[];
        unknown_fields: string[];
        options?: { hard_block_on_missing?: boolean };
      };
      expect: { kind: 'ok' | 'need_user_confirm' | 'hard_block' };
    }
  | {
      id: string;
      kind: 'execution_handler_registration_decision';
      input: {
        execution_type: string;
        core_execution_types: string[];
        registered_execution_types: string[];
        pack_plugin_allowlist?: { active?: boolean; enabled_types?: string[] };
      };
      expect: { decision: 'allow' | 'reject'; reason_code?: string };
    }
  | {
      id: string;
      kind: 'protocol_install_decision';
      input: {
        mode: 'safe' | 'assist' | 'yolo';
        source_kind: 'local_path' | 'registry_ref' | 'remote_url' | 'llm_generated';
        allowed_sources?: Array<'local_path' | 'registry_ref' | 'remote_url' | 'llm_generated'>;
        require_signature?: boolean;
        has_signature?: boolean;
      };
      expect: { decision: 'allow' | 'need_user_confirm' | 'reject'; reason_code?: string };
    };

describe('AIS conformance vectors (specs/conformance/vectors)', () => {
  const root = resolve(process.cwd(), '..'); // ts-sdk/ -> repo root
  const vectorsDir = resolve(root, 'specs', 'conformance', 'vectors');
  const goldenDir = resolve(root, 'specs', 'conformance', 'golden');
  const profilesDir = resolve(root, 'specs', 'conformance', 'profiles');

  const requestedProfile = (
    process.env.AIS_CONFORMANCE_PROFILE ?? 'core'
  ).toLowerCase();
  const files = selectVectorFiles(vectorsDir, profilesDir, requestedProfile);
  expect(files.length).toBeGreaterThan(0);

  for (const file of files) {
    const raw = readFileSync(resolve(vectorsDir, file), 'utf-8');
    const vf = JSON.parse(raw) as VectorFile;

    it(`${file}: schema`, () => {
      expect(vf.schema).toBe('ais-conformance/0.0.2');
      expect(Array.isArray(vf.cases)).toBe(true);
    });

    for (const c of vf.cases) {
      it(`${file} :: ${c.id}`, () => {
        switch (c.kind) {
          case 'jcs_canonicalize': {
            const got = canonicalizeJcs(c.input.value);
            expect(got).toBe(c.expect.canonical);
            expect(specHashKeccak256(c.input.value)).toBe(c.expect.specHashKeccak256);
            return;
          }
          case 'cel_eval': {
            const v = evaluateCEL(c.input.expression, c.input.context ?? {});
            expect(typeof v).toBe('bigint');
            expect(String(v as bigint)).toBe(c.expect.value_bigint);
            return;
          }
          case 'cel_eval_string': {
            const v = evaluateCEL(c.input.expression, c.input.context ?? {});
            expect(typeof v).toBe('string');
            expect(String(v)).toBe(c.expect.value_string);
            return;
          }
          case 'cel_eval_error': {
            let err: unknown;
            try {
              evaluateCEL(c.input.expression, c.input.context ?? {});
            } catch (e) {
              err = e;
            }
            expect(err).toBeTruthy();
            if (c.expect.message_includes) {
              expect(String((err as any)?.message ?? err)).toContain(c.expect.message_includes);
            }
            return;
          }
          case 'evm_json_abi_encode': {
            const data = encodeJsonAbiFunctionCall(c.input.abi, c.input.args);
            expect(data).toBe(c.expect.data);
            return;
          }
          case 'select_execution_spec': {
            const spec = selectExecutionSpec(c.input.execution as any, c.input.chain);
            expect((spec as any).type).toBe(c.expect.type);
            return;
          }
          case 'select_execution_spec_error': {
            let err: unknown;
            try {
              selectExecutionSpec(c.input.execution as any, c.input.chain);
            } catch (e) {
              err = e;
            }
            expect(err).toBeTruthy();
            if (c.expect.message_includes) {
              expect(String((err as any)?.message ?? err)).toContain(c.expect.message_includes);
            }
            return;
          }
          case 'workflow_plan': {
            const ctx = createContext();
            for (const y of c.input.protocols_yaml) {
              registerProtocol(ctx, parseProtocolSpec(y));
            }
            const wf = parseWorkflow(c.input.workflow_yaml);
            const plan = buildWorkflowExecutionPlan(wf, ctx);
            const normalized = normalizePlanForGolden(plan);

            const goldenPath = resolve(goldenDir, c.input.golden_file);
            const golden = JSON.parse(readFileSync(goldenPath, 'utf-8'));

            expect(normalized).toEqual(golden);
            return;
          }
          case 'confirmation_hash': {
            const hash = confirmationHash(c.input.summary);
            expect(hash).toBe(c.expect.hash_sha256_hex);
            return;
          }
          case 'json_schema_validate': {
            const valid = validateBySchemaId(c.input.schema_id, c.input.value);
            expect(valid).toBe(c.expect.valid);
            return;
          }
          case 'pack_approvals_decision': {
            const got = decidePackApprovals(c.input.approvals, c.input.risk_level);
            expect(got).toEqual(c.expect);
            return;
          }
          case 'policy_gate_missingness_decision': {
            const got = decideMissingness(c.input);
            expect(got).toEqual(c.expect);
            return;
          }
          case 'execution_handler_registration_decision': {
            const got = decideExecutionHandlerRegistration(c.input);
            expect(got).toEqual(c.expect);
            return;
          }
          case 'protocol_install_decision': {
            const got = decideProtocolInstall(c.input);
            expect(got).toEqual(c.expect);
            return;
          }
          default: {
            const neverKind: never = c;
            throw new Error(`Unknown vector kind: ${(neverKind as any).kind}`);
          }
        }
      });
    }
  }

  function normalizePlanForGolden(plan: any): any {
    // Make timestamps deterministic for golden comparison.
    const cloned = JSON.parse(JSON.stringify(plan));
    if (cloned.meta && typeof cloned.meta === 'object') {
      if ('created_at' in cloned.meta) cloned.meta.created_at = '<ignored>';
      // omit description if undefined in source (JSON.stringify already removed undefined)
    }
    return cloned;
  }

  function selectVectorFiles(
    allVectorsDir: string,
    manifestDir: string,
    profile: string
  ): string[] {
    const all = readdirSync(allVectorsDir)
      .filter((f) => f.endsWith('.json'))
      .sort();
    if (profile === 'all') {
      return all;
    }
    if (profile === 'core' || profile === 'extended') {
      const manifest = loadProfileManifest(manifestDir, profile);
      const selected = manifest.vector_files
        .map((value) => value.split('/').at(-1))
        .filter((value): value is string => Boolean(value))
        .sort();
      return selected;
    }

    throw new Error(
      `AIS_CONFORMANCE_PROFILE must be one of core|extended|all, got: ${profile}`
    );
  }

  function loadProfileManifest(
    manifestDir: string,
    profile: 'core' | 'extended'
  ): ProfileManifest {
    const filename = `${profile}-files.json`;
    const raw = readFileSync(resolve(manifestDir, filename), 'utf-8');
    const parsed = JSON.parse(raw) as ProfileManifest;
    expect(parsed.schema).toBe('ais-conformance-profile/0.0.1');
    expect(parsed.profile).toBe(profile);
    expect(Array.isArray(parsed.vector_files)).toBe(true);
    return parsed;
  }

  function validateBySchemaId(schemaId: string, value: unknown): boolean {
    switch (schemaId) {
      case 'ais-catalog/0.0.1':
        return validateCatalogSchema(value);
      case 'ais-executable-candidates/0.0.1':
        return validateExecutableCandidatesSchema(value);
      case 'ais-engine-command/0.0.1':
        return validateEngineCommandSchema(value);
      case 'ais-engine-event/0.0.3':
        return validateEngineEventSchema(value);
      case 'ais-pack/0.0.2':
        return validatePackSchema(value);
      default:
        throw new Error(`Unsupported schema_id for conformance runner: ${schemaId}`);
    }
  }

  function confirmationHash(summary: Record<string, unknown>): string {
    const normalized = stripTimestampLikeKeys(summary);
    const canonical = canonicalizeJcs(normalized);
    return createHash('sha256').update(canonical).digest('hex');
  }

  function stripTimestampLikeKeys(value: unknown): unknown {
    if (Array.isArray(value)) {
      return value.map((item) => stripTimestampLikeKeys(item));
    }
    if (!value || typeof value !== 'object') {
      return value;
    }
    const out: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
      if (key === 'ts' || key === 'timestamp' || key === 'created_at' || key === 'updated_at') {
        continue;
      }
      out[key] = stripTimestampLikeKeys(child);
    }
    return out;
  }

  function decidePackApprovals(
    approvals: {
      mode?: 'safe' | 'assist' | 'yolo';
      auto_execute_max_risk_level?: number;
      require_approval_min_risk_level?: number;
      llm_may_approve_max_risk_level?: number;
    },
    riskLevel: number
  ): { needs_confirmation: boolean; confirmer: 'none' | 'human' | 'llm' | 'auto' } {
    const mode = approvals.mode ?? 'safe';
    const autoThreshold = approvals.auto_execute_max_risk_level;
    const confirmThreshold = approvals.require_approval_min_risk_level;

    let needsConfirmation = true;
    if (typeof confirmThreshold === 'number' && riskLevel >= confirmThreshold) {
      needsConfirmation = true;
    } else if (typeof autoThreshold === 'number' && riskLevel <= autoThreshold) {
      needsConfirmation = false;
    }

    if (!needsConfirmation) {
      return { needs_confirmation: false, confirmer: 'none' };
    }

    if (mode === 'assist') {
      const llmThreshold = approvals.llm_may_approve_max_risk_level;
      if (typeof llmThreshold === 'number' && riskLevel <= llmThreshold) {
        return { needs_confirmation: true, confirmer: 'llm' };
      }
      return { needs_confirmation: true, confirmer: 'human' };
    }
    if (mode === 'yolo') {
      return { needs_confirmation: true, confirmer: 'auto' };
    }
    return { needs_confirmation: true, confirmer: 'human' };
  }

  function decideMissingness(input: {
    hard_block_fields: string[];
    missing_fields: string[];
    unknown_fields: string[];
    options?: { hard_block_on_missing?: boolean };
  }): { kind: 'ok' | 'need_user_confirm' | 'hard_block' } {
    if (input.hard_block_fields.length > 0) {
      return { kind: 'hard_block' };
    }
    if (input.missing_fields.length > 0) {
      if (input.options?.hard_block_on_missing === true) {
        return { kind: 'hard_block' };
      }
      return { kind: 'need_user_confirm' };
    }
    if (input.unknown_fields.length > 0) {
      return { kind: 'need_user_confirm' };
    }
    return { kind: 'ok' };
  }

  function decideExecutionHandlerRegistration(input: {
    execution_type: string;
    core_execution_types: string[];
    registered_execution_types: string[];
    pack_plugin_allowlist?: { active?: boolean; enabled_types?: string[] };
  }): { decision: 'allow' | 'reject'; reason_code?: string } {
    const isRegistered = input.registered_execution_types.includes(input.execution_type);
    if (!isRegistered) {
      return { decision: 'reject', reason_code: 'unregistered_execution_handler' };
    }

    const isCoreType = input.core_execution_types.includes(input.execution_type);
    if (!isCoreType && input.pack_plugin_allowlist?.active) {
      const allowedTypes = input.pack_plugin_allowlist.enabled_types ?? [];
      if (!allowedTypes.includes(input.execution_type)) {
        return { decision: 'reject', reason_code: 'plugin_execution_type_not_allowlisted' };
      }
    }
    return { decision: 'allow' };
  }

  function decideProtocolInstall(input: {
    mode: 'safe' | 'assist' | 'yolo';
    source_kind: 'local_path' | 'registry_ref' | 'remote_url' | 'llm_generated';
    allowed_sources?: Array<'local_path' | 'registry_ref' | 'remote_url' | 'llm_generated'>;
    require_signature?: boolean;
    has_signature?: boolean;
  }): { decision: 'allow' | 'need_user_confirm' | 'reject'; reason_code?: string } {
    const allowed = input.allowed_sources ?? [];
    if (!allowed.includes(input.source_kind)) {
      return { decision: 'reject', reason_code: 'source_kind_not_allowed' };
    }

    if (input.require_signature && input.source_kind !== 'local_path' && input.has_signature !== true) {
      return { decision: 'reject', reason_code: 'signature_required' };
    }

    if (input.mode === 'assist' && input.source_kind !== 'local_path') {
      return { decision: 'need_user_confirm', reason_code: 'assist_dynamic_source_requires_confirm' };
    }

    return { decision: 'allow' };
  }

  function validateCatalogSchema(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (value.schema !== 'ais-catalog/0.0.1') return false;
    if (!isSha256Hex(value.hash)) return false;
    if (!Array.isArray(value.actions) || !Array.isArray(value.queries) || !Array.isArray(value.packs)) return false;
    if (!onlyKeys(value, ['schema', 'created_at', 'hash', 'documents', 'actions', 'queries', 'packs', 'extensions'])) return false;

    for (const action of value.actions) {
      if (!isCatalogActionCard(action)) return false;
    }
    for (const query of value.queries) {
      if (!isCatalogQueryCard(query)) return false;
    }
    for (const pack of value.packs) {
      if (!isCatalogPackCard(pack)) return false;
    }
    return true;
  }

  function validateExecutableCandidatesSchema(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (value.schema !== 'ais-executable-candidates/0.0.1') return false;
    if (!isSha256Hex(value.hash) || !isSha256Hex(value.catalog_hash)) return false;
    if (!isString(value.catalog_schema)) return false;
    if (!Array.isArray(value.actions) || !Array.isArray(value.queries)) return false;
    if (!Array.isArray(value.detect_providers) || !Array.isArray(value.execution_plugins)) return false;
    if (
      !onlyKeys(value, [
        'schema',
        'created_at',
        'hash',
        'catalog_schema',
        'catalog_hash',
        'pack',
        'chain_scope',
        'actions',
        'queries',
        'detect_providers',
        'execution_plugins',
        'extensions',
      ])
    ) {
      return false;
    }

    for (const action of value.actions) {
      if (!isExecutableActionCard(action)) return false;
    }
    for (const query of value.queries) {
      if (!isExecutableQueryCard(query)) return false;
    }
    for (const provider of value.detect_providers) {
      if (!isExecutableDetectProvider(provider)) return false;
    }
    for (const plugin of value.execution_plugins) {
      if (!isExecutablePlugin(plugin)) return false;
    }
    return true;
  }

  function validateEngineCommandSchema(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (value.schema !== 'ais-engine-command/0.0.1') return false;
    if (!onlyKeys(value, ['schema', 'command'])) return false;
    if (!isObject(value.command) || !onlyKeys(value.command, ['id', 'type', 'data'])) return false;
    if (!isNonEmptyString(value.command.id) || !isString(value.command.type)) return false;

    const data = value.command.data;
    switch (value.command.type) {
      case 'apply_patches':
        return (
          isObject(data) &&
          onlyKeys(data, ['patches']) &&
          Array.isArray(data.patches) &&
          data.patches.length > 0 &&
          data.patches.every((patch) => isRuntimePatch(patch))
        );
      case 'user_confirm':
        return (
          isObject(data) &&
          onlyKeys(data, ['node_id', 'decision']) &&
          isNonEmptyString(data.node_id) &&
          (data.decision === 'approve' || data.decision === 'deny')
        );
      case 'select_provider':
        return isObject(data) && onlyKeys(data, ['provider']) && isNonEmptyString(data.provider);
      case 'cancel':
        return data === undefined || (isObject(data) && onlyKeys(data, ['reason']) && (data.reason === undefined || isString(data.reason)));
      case 'replace_plan':
        return (
          isObject(data) &&
          onlyKeys(data, ['plan', 'reason']) &&
          isObject(data.plan) &&
          data.plan.schema === 'ais-plan/0.0.3' &&
          Array.isArray(data.plan.nodes) &&
          (data.reason === undefined || isString(data.reason))
        );
      default:
        return false;
    }
  }

  function validateEngineEventSchema(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (value.schema !== 'ais-engine-event/0.0.3') return false;
    if (!isNonEmptyString(value.run_id) || !Number.isInteger(value.seq) || value.seq < 0 || !isNonEmptyString(value.ts)) return false;
    if (!onlyKeys(value, ['schema', 'run_id', 'seq', 'ts', 'event'])) return false;
    if (!isObject(value.event)) return false;
    if (!onlyKeys(value.event, ['type', 'node_id', 'data', 'extensions'])) return false;
    if (!isString(value.event.type) || !isObject(value.event.data)) return false;
    if (value.event.node_id !== undefined && !isString(value.event.node_id)) return false;

    if (value.event.type === 'command_accepted' || value.event.type === 'command_rejected') {
      const data = value.event.data;
      if (!isString(data.command_id) || !isString(data.command_type)) return false;
      if (typeof data.duplicate !== 'boolean' || typeof data.noop !== 'boolean') return false;
    }
    if (value.event.type === 'plan_replaced') {
      const data = value.event.data;
      if (!isString(data.before_plan_hash) || !isString(data.after_plan_hash)) return false;
    }
    return true;
  }

  function validatePackSchema(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (value.schema !== 'ais-pack/0.0.2') return false;
    if (!isObject(value.meta) || !isNonEmptyString(value.meta.name) || !isString(value.meta.version)) return false;
    if (!Array.isArray(value.includes)) return false;
    if (!onlyKeys(value, ['schema', 'meta', 'includes', 'policy', 'token_policy', 'providers', 'plugins', 'overrides', 'extensions'])) {
      return false;
    }
    if (!value.policy) return true;
    if (!isObject(value.policy)) return false;
    if (
      !onlyKeys(value.policy, [
        'approvals',
        'protocol_install',
        'constraints',
        'extensions',
      ])
    ) {
      return false;
    }
    if (value.policy.approvals && !isPackApprovals(value.policy.approvals)) return false;
    if (value.policy.constraints && !isPackConstraints(value.policy.constraints)) return false;
    return true;
  }

  function isPackApprovals(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (!onlyKeys(value, ['mode', 'auto_execute_max_risk_level', 'require_approval_min_risk_level', 'llm_may_approve_max_risk_level', 'extensions'])) {
      return false;
    }
    if (value.mode !== undefined && value.mode !== 'safe' && value.mode !== 'assist' && value.mode !== 'yolo') return false;
    return true;
  }

  function isPackConstraints(value: unknown): boolean {
    if (!Array.isArray(value)) return false;
    for (const item of value) {
      if (!isObject(item)) return false;
      if (!onlyKeys(item, ['id', 'effect', 'expr', 'message', 'extensions'])) return false;
      if (!isNonEmptyString(item.id) || !isNonEmptyString(item.expr)) return false;
      if (item.effect !== 'hard_block' && item.effect !== 'need_user_confirm') return false;
      if (item.message !== undefined && !isString(item.message)) return false;
    }
    return true;
  }

  function isCatalogActionCard(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (
      !onlyKeys(value, [
        'level',
        'ref',
        'protocol',
        'version',
        'id',
        'description',
        'risk_level',
        'risk_tags',
        'capabilities_required',
        'execution_types',
        'execution_chains',
        'extensions',
      ])
    ) {
      return false;
    }
    if (!isNonEmptyString(value.ref) || !isNonEmptyString(value.protocol) || !isNonEmptyString(value.version) || !isNonEmptyString(value.id)) return false;
    if (!isRiskLevel(value.risk_level)) return false;
    if (!isStringArray(value.execution_types) || !isStringArray(value.execution_chains)) return false;
    if (value.level !== undefined && value.level !== 'index') return false;
    return true;
  }

  function isCatalogQueryCard(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (
      !onlyKeys(value, [
        'level',
        'ref',
        'protocol',
        'version',
        'id',
        'description',
        'capabilities_required',
        'execution_types',
        'execution_chains',
        'extensions',
      ])
    ) {
      return false;
    }
    if (!isNonEmptyString(value.ref) || !isNonEmptyString(value.protocol) || !isNonEmptyString(value.version) || !isNonEmptyString(value.id)) return false;
    if (!isStringArray(value.execution_types) || !isStringArray(value.execution_chains)) return false;
    if (value.level !== undefined && value.level !== 'index') return false;
    return true;
  }

  function isCatalogPackCard(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (!onlyKeys(value, ['name', 'version', 'description', 'includes', 'policy', 'token_policy', 'providers', 'plugins', 'overrides', 'extensions'])) {
      return false;
    }
    return isString(value.name) && isString(value.version) && Array.isArray(value.includes);
  }

  function isExecutableActionCard(value: unknown): boolean {
    return isCatalogActionCard(value);
  }

  function isExecutableQueryCard(value: unknown): boolean {
    return isCatalogQueryCard(value);
  }

  function isExecutableDetectProvider(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (!onlyKeys(value, ['kind', 'provider', 'chain', 'priority', 'extensions'])) return false;
    return isNonEmptyString(value.kind) && isNonEmptyString(value.provider);
  }

  function isExecutablePlugin(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (!onlyKeys(value, ['type', 'chain', 'extensions'])) return false;
    return isNonEmptyString(value.type);
  }

  function isRuntimePatch(value: unknown): boolean {
    if (!isObject(value)) return false;
    if (!onlyKeys(value, ['op', 'path', 'value', 'extensions'])) return false;
    if (value.op !== 'set' && value.op !== 'merge') return false;
    return isNonEmptyString(value.path);
  }

  function onlyKeys(value: Record<string, unknown>, keys: string[]): boolean {
    return Object.keys(value).every((key) => keys.includes(key));
  }

  function isObject(value: unknown): value is Record<string, unknown> {
    return !!value && typeof value === 'object' && !Array.isArray(value);
  }

  function isString(value: unknown): value is string {
    return typeof value === 'string';
  }

  function isNonEmptyString(value: unknown): value is string {
    return typeof value === 'string' && value.length > 0;
  }

  function isStringArray(value: unknown): value is string[] {
    return Array.isArray(value) && value.every((item) => typeof item === 'string');
  }

  function isRiskLevel(value: unknown): boolean {
    return Number.isInteger(value) && (value as number) >= 1 && (value as number) <= 5;
  }

  function isSha256Hex(value: unknown): boolean {
    return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
  }
});
