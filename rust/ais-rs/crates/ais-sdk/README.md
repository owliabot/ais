# `ais-sdk`

Pure AIS SDK logic: document parsing, typed models, resolver context, and value-ref evaluation.

## Responsibility

- Parse AIS JSON/YAML with schema-based dispatch
- Detect YAML duplicate keys (safety)
  - Duplicate keys are scoped per mapping object (including array item mappings), so repeated keys across different list items are allowed.
- Typed top-level document structs for protocol/pack/workflow/plan/catalog/plan-skeleton
  - plus segmented intent planning IR: `plan-sketch`
- Resolver context (`runtime` + protocol/pack registry)
- Protocol deployment resolution by chain, with selected deployment metadata/`contracts` projection for compiled plan nodes
- ValueRef sync/async evaluation (`lit/ref/cel/object/array`) with root overrides
- Resolver path bridge for asset compatibility: `.address` reads are accepted when the source slot already holds a raw address string (for example `params.token = "0x..."` still satisfies `params.token.address`)
- Protocol/action/query reference parsing and resolution (`protocol@version/action|query`)
- Catalog build (`ais-catalog/0.0.1`) with index/detail card levels, spec-aligned card fields, stable sorting and hash
- Executable candidates projection (`ais-executable-candidates/0.0.1`) with action/query cards + execution plugin candidates
- PlanSkeleton compile to `ais-plan/0.0.3` + synthesized workflow

## Public entry points

- Parse:
  - `parse_document`
  - `parse_document_with_options`
  - `AisDocument`
- Documents:
  - `ProtocolDocument`, `PackDocument`, `WorkflowDocument`
  - `PlanDocument`, `CatalogDocument`, `PlanSkeletonDocument`, `PlanSketchDocument`
- Resolver:
  - `ResolverContext`
  - `ResolvedNodeBindings`
  - `ResolvedQueryBindings`
  - `CalculatedBindingsResult`
  - `ValueRef`
  - `evaluate_value_ref`
  - `evaluate_value_ref_with_options`
  - `evaluate_value_ref_async`
  - `resolve_calculated_bindings`
  - `resolve_calculated_bindings_async`
  - `resolve_node_bindings`
  - `resolve_query_bindings`
  - `parse_action_ref` / `parse_query_ref`
  - `resolve_action_ref` / `resolve_query_ref`
  - `ActionRef` / `QueryRef`
  - `calculated_override_order`
  - `calculated_override_order_from_map`
  - `CalculatedOverrideError`
- Protocol:
  - `resolve_deployment_for_chain`
  - `resolve_operation_spec`
  - `token_resolution_policy`
  - `resolve_token_candidate_for_symbol`
  - `resolve_token_candidate_for_address`
  - `build_protocol_extension`
  - `build_pack_extension`
  - `ResolvedDeployment`
  - `ResolvedOperationKind`
  - `ResolvedOperationSpec`
  - `ResolvedPackOperation`
  - `ResolvedTokenCandidate`
  - `TokenResolutionPolicy`
  - `TokenResolutionError`
  - `TokenResolutionErrorCode`
- Validate:
  - `validate_document_semantics`
  - `validate_workspace_references`
  - `validate_workflow_document`
  - `WorkspaceDocuments`
- Catalog:
  - `build_catalog`
  - `build_catalog_index`
  - `filter_by_pack`
  - `filter_by_engine_capabilities`
  - `get_executable_candidates`
  - `CatalogBuildInput`
  - `CatalogBuildOptions`
  - `CatalogCardLevel`
  - `ActionCard`
  - `QueryCard`
  - `CatalogParam`
  - `CatalogReturn`
  - `CatalogIndex`
  - `EngineCapabilities`
  - `ExecutableCandidates`
- Planner:
  - `compile_plan_skeleton`
  - `compile_plan_sketch`
  - `compile_workflow`
  - `dry_run_json`
  - `dry_run_json_async`
  - `dry_run_text`
  - `dry_run_text_async`
  - `render_dry_run_text`
  - `get_node_readiness`
  - `get_node_readiness_async`
  - `CompilePlanSkeletonOptions`
  - `CompilePlanSkeletonResult`
  - `CompilePlanSketchOptions`
  - `CompilePlanSketchResult`
  - `CompileWorkflowOptions`
  - `CompileWorkflowResult`
  - `DryRunSummary`
  - `DryRunNodeReport`
  - `DryRunJsonReport`
  - `NodeRunState`
  - `NodeReadinessResult`
  - `value_ref_eval_options_for_node`

## Recent changes

- `CPS-501 token resolution contract`: `protocol.rs` now exposes pack-aware token resolution helpers for segmented planner/agent callers. `token_resolution_policy(...)`, `resolve_token_candidate_for_symbol(...)`, and `resolve_token_candidate_for_address(...)` merge pack `token_policy`, pack allowlist entries, protocol `supported_assets`, and chain context into deterministic `ResolvedTokenCandidate` / `TokenResolutionError` results before plan compilation.
- `F-PROJ-01`: `context::get_ref` now unwraps `_value` sentinel keys when resolution yields an object containing a `_value` field, supporting the InputStore leaf/subtree collision preservation pattern. Address bridge compatibility extended to handle `_value` sentinel in intermediate objects.
- `F-PROJ-02`: `compile_workflow` and `compile_plan_sketch` now resolve the selected protocol deployment for the target chain and persist `extensions.protocol.{ref,deployment_chain,deployment,contracts}` on each compiled node. Readiness/eval helpers now project `contracts.*` from node extensions into ValueRef root overrides so `to: { ref: "contracts.pool" }` can resolve without ad hoc runtime injection.
- `POST-005 deployment contracts validation`: protocol documents and workspace loading now reject non-object `deployments[].contracts` with structured issues instead of silently degrading to an empty contract map. `resolve_deployment_for_chain(...)` also skips malformed deployment entries rather than materializing invalid `contracts.*` bindings.
- `POST-006 dropped providers residue removal`: resolved pack semantics no longer carry `providers` through `ResolvedPackOperation` or compiled `extensions.pack`. `pack.providers` remains a document field only, but it is no longer part of the active compile/runtime semantic surface.
- `POST-007 node-scoped runtime controls`: `get_node_readiness` now pre-resolves node `bindings.params` plus `calculated_overrides` before evaluating `condition`, so readiness gating sees the same node-scoped `params.*` / `calculated.*` roots as downstream execution materialization instead of relying on global runtime pre-seeding.
- `POST-008 pack snapshot invariant`: workflow compile now fails closed when `requires_pack` cannot be resolved from `ResolverContext`, and plan-sketch compile now verifies `pack_snapshot.{name,version,hash}` against the actually loaded pack document before lowering any node. Nodes therefore cannot claim `extensions.pack.hash` for semantics compiled from a different pack snapshot.
- `CPS-202 pack merge`: `resolve_operation_spec(...)` now resolves protocol action/query semantics through deployment + optional pack merge before execution selection. `compile_workflow` and `compile_plan_sketch` both compile against the merged spec, persist `extensions.pack` audit metadata, and surface pack-selected description / risk / `requires_queries` / constraint provenance from a single resolution path.
- `CPS-203 resolved metadata snapshot`: compiled workflow/sketch nodes now also persist `extensions.operation` as the stable merged operation identity (`protocol_ref`, `kind`, `key`, `selector`, `target_chain`) and enrich `extensions.pack` with snapshot identity fields (`name`, `version`, `hash`) when available, so audit/debug can trace exactly which resolved protocol/pack snapshot produced a node.
- `CPS-301 resolved node bindings`: shared root materialization now lives in `ResolvedNodeBindings` / `resolve_node_bindings(...)`. `get_node_readiness` and engine-side execution/condition/assert materialization consume the same binding builder for `params`, deployment-backed `contracts`, node `policy`, and runtime `query` / `calculated` roots.
- `CPS-302 calculated-field lowering`: protocol action/query `calculated_fields` now lower into compiled node `calculated_overrides`, and `resolve_calculated_bindings(...)` evaluates them in dependency order against the shared binding roots. Readiness and engine materialization therefore resolve `calculated.*` from protocol semantics directly instead of relying on host-side precomputation.
- `CPS-303 query binding contract`: protocol/pack `requires_queries` now lower into `extensions.operation.requires_queries`, and compilers persist dependency-backed `extensions.operation.query_bindings` when an action step depends on an explicit query producer. `resolve_query_bindings(...)` merges those prerequisite node outputs with any existing runtime `query` root, so readiness/engine can satisfy `query.<name>.*` and CEL `query["name"]` reads from one normalized source.
- `CPS-304 policy metadata lowering`: `build_policy_extension(...)` now lowers pack-derived constraint arrays into `extensions.policy`. Compiled nodes carry ordered `global_constraints` / `action_rule_constraints` / `action_constraints` / `effective_constraints`, while preserving `param_roles` / `required_fields` metadata for downstream policy-gate extraction. Legacy protocol-level `hard_constraints` are no longer part of the active protocol surface.
- `CPS-401 composite lowering`: planner compile now rewrites protocol `execution.type = composite` into ordered base plan nodes before executor handoff. `lower_composite_node(...)` converts `steps[]` into deterministic node ids, preserves dependency order, annotates each lowered node with `extensions.composite.{parent_node_id,step_id,step_index,step_count}`, and ensures example-backed green paths no longer emit raw `composite` execution to runtime.
- `CPS-402 composite metadata preservation`: composite lowering now also rewrites local step output refs (for example `nodes.approve.outputs.*`) into lowered node ids, adds `source.composite_step_id`, enriches `extensions.composite` with output/local-step mapping metadata, and preserves segmented planner provenance via `extensions.plan_sketch.composite_step_id` while keeping step-level `stores` attached only to the final lowered node.
- `CPS-403 composite policy overlay`: lowered composite steps now keep parent action risk/policy metadata by default, but approval-like sub-steps (`approve`) receive a step-local overlay. That overlay appends `approval` to `extensions.risk_tags`, preserves the original risk level, annotates `extensions.composite.semantic_kind`, and maps policy-gate roles `spender_address -> spender` / `approval_amount -> amount` so engine policy checks can reason about the actual approval call without losing pack/protocol constraints on the downstream action step.

## Dependencies

- `ais-core`: issues / field-path / patch primitives
- `ais-schema`: schema constants + validation adapter
- `ais-cel`: CEL lexer/parser/numeric/evaluator for `ValueRef::Cel`
- `num-bigint`: bridge `CelValue::Integer(BigInt)` to JSON-safe output (number when in range, string otherwise)

## Test layout

- Unit tests live in dedicated `*_test.rs` files inside each module directory.
- Workspace validation includes fixture-backed tests from `rust/ais-rs/fixtures/workspace-minimal`.
- Workflow imports validation includes fixture-backed tests from `rust/ais-rs/fixtures/workflow-0.0.3/imports`.
- Workflow assert compile/validation checks include fixture-backed tests from `rust/ais-rs/fixtures/workflow-0.0.3/assert`.
- Workflow calculated_overrides checks include fixture-backed tests from `rust/ais-rs/fixtures/workflow-0.0.3/calculated_overrides`.

## Complex Protocol Support Boundaries

- Example-backed green paths now cover:
  - `aave-v3.supply-raw`: deployment-selected `contracts.*`
  - `aave-v3.withdraw`: protocol `calculated_fields`
  - `aave-v3.supply`: `requires_queries` + composite lowering into base nodes
  - `safe-defi-pack` constrained `uniswap-v3.swap-exact-in`: pack merge, constraint lowering, token policy contract, and prerequisite query bindings
- Compiler/runtime boundary is now explicit:
  - protocol `deployments`, pack overrides/constraints, protocol `calculated_fields`, and `execution.type = composite` are lowered before executor handoff
  - executor-facing plans should only see base execution nodes plus resolved metadata under `extensions.*`
- Raw example ingestion now works for the main target files:
  - YAML duplicate-key safety no longer misclassifies namespaced chain-map keys such as `eip155:1:` inside protocol examples
  - pack schema now ingests `policy.execution.volatile_facts.max_age_ms` from raw pack examples
  - workspace/runner can therefore parse, schema-validate, and ingest raw `examples/aave-v3.ais.yaml`, `examples/uniswap-v3.ais.yaml`, and `examples/safe-defi-pack.ais-pack.yaml` without a pre-normalization pass
- Protocol asset registry shape now supports both:
  - legacy chain-map form: `addresses.{chain}` + `decimals.{chain}`
  - preferred slim form: `supported_assets[].deployments[] = { chain, address, decimals }`
  `resolve_token_candidate_for_symbol/address(...)` reads both, so examples can migrate without breaking existing workspace content.
- Reduced-semantic fixtures still remain in runner coverage where downstream execute proof intentionally simplifies raw detect/slippage surfaces; raw ingestion parity is broader than raw execution parity

## Current status

- Implemented:
  - `AISRS-SDK-001`
  - `AISRS-SDK-010`
  - `AISRS-SDK-011`
  - `AISRS-SDK-020`
  - `AISRS-SDK-021`
  - `AISRS-SDK-022` (root_overrides + CEL evaluation wired)
  - `AISRS-SDK-023` (reference parsing + protocol/action/query resolution)
  - `AISRS-SDK-030` (single-document semantic validation with stable field paths)
  - `AISRS-SDK-031` (workspace validation for requires_pack/includes/chain_scope/protocol refs)
  - `AISRS-SDK-032` (workflow validation for DAG/deps/ValueRef refs)
  - `AISRS-SDK-033` (workflow imports semantic validation + workspace closure checks)
  - `AISRS-SDK-034` (workflow assert/assert_message compile+validation semantics)
  - `AISRS-SDK-035` (calculated_overrides dependency ordering + missing/cycle diagnostics)
  - `AISRS-SDK-040` (catalog build with stable sort and hash)
  - `AISRS-CAT-001` (catalog cards aligned to spec fields: risk/description/detail params+returns, index/detail levels, stable hash)
  - `AISRS-SDK-041` (catalog index + pack/engine filters, including wildcard chain matching between `execution_chains` and pack `chain_scope`, e.g. `eip155:*` ↔ `eip155:31338`)
  - hard-delete `detect_providers` from executable candidates and keep plugin-only capability projection (`execution_plugins`)
  - `AISRS-SDK-050` (compile plan skeleton into execution plan + workflow)
  - `AISNEXT-RS-001` foundation:
    - parse/dispatch support for `ais-plan-sketch/0.1.0` via `AisDocument::PlanSketch`
    - typed `PlanSketchDocument` model for segmented planner output
    - semantic validation hook for non-empty `segments/steps`
  - `AISNEXT-RS-002` foundation:
    - deterministic `compile_plan_sketch` path from `PlanSketchDocument` to `ais-plan/0.0.3`
    - stable canonical node id mapping (`segment_id__step_id`, CEL-safe ASCII) and deterministic param binding wrapping (`lit` passthrough/value wrapping)
    - compiler rewrites node references in step inputs/conditions/runtime-controls to canonical ids (`when.cel`, `until`, nested ValueRef `ref/cel`) so runtime `nodes.*.outputs` paths stay consistent
    - node reference contract is strict-local: `nodes.<step_id>.*` must resolve to steps in the same segment; cross-segment `segment/step` refs are rejected as compile issues (`non_local_node_ref`/`unknown_node_ref`)
    - asset-param normalization for segmented steps: `asset` inputs given as address string / `{lit:"0x..."}` or object with `chain_ref` are normalized into object ValueRef (`address/chain_id`, with `chain_ref -> chain_id`) so execution refs like `params.token.address` resolve deterministically
    - step-level `constraint_templates[]` are copied into `node.extensions.policy.constraint_templates` for downstream policy-gate enforcement
    - step-level `stores` are preserved at `node.extensions.plan_sketch.stores` for runner-side fact backfill/audit mapping
    - segment-level `extensions.todo_id` is propagated into each compiled node at `node.extensions.plan_sketch.todo_id` for stable todo/segment/node traceability
    - executable-candidate risk metadata is propagated into compiled step nodes when available:
      - `candidate.{risk_level,risk_tags} -> node.extensions.{risk_level,risk_tags}` for `action`/`query` nodes
      - missing risk metadata is treated as optional (field omitted; existing extensions shape unchanged)
    - step-level runtime controls are supported and compiled deterministically: `until -> node.until`, `retry -> node.retry`, `timeout_ms -> node.timeout_ms`
    - compiler validates runtime control shapes (`until` ValueRef parse, positive `retry.interval_ms` / `retry.max_attempts` / `timeout_ms`) and emits `input_type_mismatch` on invalid values
    - structured compile issues for core reason codes:
      - `candidate_not_found`
      - `candidate_chain_not_allowed`
      - `execution_type_not_allowed`
      - `missing_required_input`
      - `input_type_mismatch`
      - `unknown_input_ref` (when `CompilePlanSketchOptions.known_input_refs` is provided and step ValueRef uses unknown `inputs.*` path)
    - plan-sketch control kinds `assert`/`branch` are built-in compile-time control semantics:
      - they do not require `candidate_ref`
      - they are not emitted as executable runtime nodes
      - downstream executable step dependencies are flattened through control steps
      - control conditions (`when.cel` or `inputs.condition.cel`) are merged into downstream executable `condition.cel`
  - `AISNEXT-TEST-001`:
    - compile canary snapshot test for `plan-sketch -> plan` deterministic mapping
    - fixed canary hash to detect unintentional compiler shape changes
  - `AISRS-SDK-051` (compile workflow into execution plan with stable topological order + workflow preflight passthrough into plan meta)
  - `compile_workflow` and `compile_plan_skeleton` now accept control-intent node types (`assert`/`branch`) and lower them to executable `query_ref`/`action_ref` by resolving exactly one target leaf (`query` or `action`); control intent is retained at `extensions.control.step_kind`
  - control-intent nodes in workflow/skeleton reject ambiguous or missing targets with structured issues (`*.node.control_target_ambiguous` / `*.node.control_target_required`)
  - `compile_workflow` now copies action `risk_level`/`risk_tags` into plan node `extensions` for policy gate + confirmation UX, and emits `extensions.policy.{param_roles,required_fields}` derived from action schemas with canonical role names only (`spend_amount`, `slippage_bps`, `spender_address`, `approval_amount`), without alias inference.
  - `AISRS-SDK-052` (node readiness: missing refs / condition skipped)
  - `AISRS-SDK-053` (dry-run text/json with per-node report, issues, stable hashes)
  - deployment-aware plan compilation/runtime binding:
    - `compile_workflow` and `compile_plan_sketch` now require a matching protocol deployment for the selected chain
    - compiled nodes carry selected deployment metadata under `extensions.protocol`
    - `get_node_readiness` and shared node eval helpers inject `contracts.*` from `extensions.protocol.contracts`
  - pack-aware operation resolution:
    - `ResolverContext` now indexes packs by `name@version`
    - `resolve_operation_spec` merges `policy.constraints[]`, matching `overrides.action_rules[]`, and `overrides.actions.<selector>`
    - workflow/sketch compilers select execution and copy metadata from the merged operation spec instead of the raw protocol spec
    - compiled nodes persist `extensions.operation` and `extensions.pack` with merged identity/constraint provenance for downstream audit/debug
  - shared node binding contract:
    - `resolve_node_bindings` produces the common runtime-visible roots consumed by readiness and engine execution
    - current shared roots are `params`, `contracts`, `policy`, plus runtime-projected `query` / `calculated` when present
  - multi-chain plan-sketch compile contract:
    - `PlanSketchStep` may now carry explicit `chain`
    - `compile_plan_sketch` no longer requires one implicit segment-wide default chain when every executable step supplies its own chain
    - multi-chain `chain_scope` now fails with structured diagnostics when a query/action step omits `chain` or names a chain outside the declared scope
  - composite execution lowering:
    - `compile_workflow` and `compile_plan_sketch` lower protocol `execution.type = composite` into base nodes before plan emission
    - intermediate lowered steps inherit node metadata, honor step-local `steps[].chain` overrides, and rebind deployment-backed `contracts.*` / `extensions.operation.target_chain` against each lowered step's effective chain
    - lowered nodes persist step-local ids/deps under `extensions.composite`
    - compiler paths now re-run node-id uniqueness validation after lowering, so emitted ids such as `<base>__approve` cannot silently collide with pre-existing workflow/sketch node ids
    - executor-facing plans therefore only contain base execution kinds such as `evm_call`
  - Minor cleanup: simplified optional/object handling and chain-scope filtering code paths (`?`/`contains`) for clearer semantics.
- Planned next:
  - `AISRS-ENG-001+` engine events/loop integration
