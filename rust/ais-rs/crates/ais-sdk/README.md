# `ais-sdk`

Pure AIS SDK logic: document parsing, typed models, resolver context, and value-ref evaluation.

## Responsibility

- Parse AIS JSON/YAML with schema-based dispatch
- Detect YAML duplicate keys (safety)
  - Duplicate keys are scoped per mapping object (including array item mappings), so repeated keys across different list items are allowed.
- Typed top-level document structs for protocol/pack/workflow/plan/catalog/plan-skeleton
  - plus segmented intent planning IR: `plan-sketch`
- Resolver context (`runtime` + protocol registry)
- ValueRef sync/async evaluation (`lit/ref/cel/object/array`) with root overrides
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
  - `ValueRef`
  - `evaluate_value_ref`
  - `evaluate_value_ref_with_options`
  - `evaluate_value_ref_async`
  - `parse_action_ref` / `parse_query_ref`
  - `resolve_action_ref` / `resolve_query_ref`
  - `ActionRef` / `QueryRef`
  - `calculated_override_order`
  - `calculated_override_order_from_map`
  - `CalculatedOverrideError`
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
    - step-level runtime controls are supported and compiled deterministically: `until -> node.until`, `retry -> node.retry`, `timeout_ms -> node.timeout_ms`
    - compiler validates runtime control shapes (`until` ValueRef parse, positive `retry.interval_ms` / `retry.max_attempts` / `timeout_ms`) and emits `input_type_mismatch` on invalid values
    - structured compile issues for core reason codes:
      - `candidate_not_found`
      - `candidate_chain_not_allowed`
      - `execution_type_not_allowed`
      - `missing_required_input`
      - `input_type_mismatch`
      - `unknown_input_ref` (when `CompilePlanSketchOptions.known_input_refs` is provided and step ValueRef uses unknown `inputs.*` path)
    - plan-sketch step kinds `assert`/`branch` are accepted in compile path by resolving `candidate_ref` kind from discovered candidates and lowering to executable `query_ref`/`action_ref` nodes; original control-kind is preserved at `node.extensions.plan_sketch.step_kind` for tracing
    - control-kind (`assert`/`branch`) compile currently requires discovery candidates context; if candidates are absent or ref not discovered, compile emits `candidate_not_found`
  - `AISNEXT-TEST-001`:
    - compile canary snapshot test for `plan-sketch -> plan` deterministic mapping
    - fixed canary hash to detect unintentional compiler shape changes
  - `AISRS-SDK-051` (compile workflow into execution plan with stable topological order + workflow preflight passthrough into plan meta)
  - `compile_workflow` and `compile_plan_skeleton` now accept control-intent node types (`assert`/`branch`) and lower them to executable `query_ref`/`action_ref` by resolving exactly one target leaf (`query` or `action`); control intent is retained at `extensions.control.step_kind`
  - control-intent nodes in workflow/skeleton reject ambiguous or missing targets with structured issues (`*.node.control_target_ambiguous` / `*.node.control_target_required`)
  - `compile_workflow` now copies action `risk_level`/`risk_tags` into plan node `extensions` for policy gate + confirmation UX, and emits `extensions.policy.{param_roles,required_fields}` derived from action schemas with canonical role names only (`spend_amount`, `slippage_bps`, `spender_address`, `approval_amount`), without alias inference.
  - `AISRS-SDK-052` (node readiness: missing refs / condition skipped)
  - `AISRS-SDK-053` (dry-run text/json with per-node report, issues, stable hashes)
  - Minor cleanup: simplified optional/object handling and chain-scope filtering code paths (`?`/`contains`) for clearer semantics.
- Planned next:
  - `AISRS-ENG-001+` engine events/loop integration
