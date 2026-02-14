# `ais-sdk`

Pure AIS SDK logic: document parsing, typed models, resolver context, and value-ref evaluation.

## Responsibility

- Parse AIS JSON/YAML with schema-based dispatch
- Detect YAML duplicate keys (safety)
  - Duplicate keys are scoped per mapping object (including array item mappings), so repeated keys across different list items are allowed.
- Typed top-level document structs for protocol/pack/workflow/plan/catalog/plan-skeleton
- Resolver context (`runtime` + protocol registry)
- ValueRef sync/async evaluation with detect-resolver trait and root overrides
- Protocol/action/query reference parsing and resolution (`protocol@version/action|query`)
- Catalog build (`ais-catalog/0.0.1`) with stable sorting and hash
- PlanSkeleton compile to `ais-plan/0.0.3` + synthesized workflow

## Public entry points

- Parse:
  - `parse_document`
  - `parse_document_with_options`
  - `AisDocument`
- Documents:
  - `ProtocolDocument`, `PackDocument`, `WorkflowDocument`
  - `PlanDocument`, `CatalogDocument`, `PlanSkeletonDocument`
- Resolver:
  - `ResolverContext`
  - `ValueRef`
  - `evaluate_value_ref`
  - `evaluate_value_ref_with_options`
  - `evaluate_value_ref_async`
  - `DetectResolver`
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
  - `CatalogIndex`
  - `EngineCapabilities`
  - `ExecutableCandidates`
- Planner:
  - `compile_plan_skeleton`
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
  - `AISRS-SDK-022` (detect/root_overrides + CEL evaluation wired)
  - `AISRS-SDK-023` (reference parsing + protocol/action/query resolution)
  - `AISRS-SDK-030` (single-document semantic validation with stable field paths)
  - `AISRS-SDK-031` (workspace validation for requires_pack/includes/chain_scope/protocol refs)
  - `AISRS-SDK-032` (workflow validation for DAG/deps/ValueRef refs)
  - `AISRS-SDK-033` (workflow imports semantic validation + workspace closure checks)
  - `AISRS-SDK-034` (workflow assert/assert_message compile+validation semantics)
  - `AISRS-SDK-035` (calculated_overrides dependency ordering + missing/cycle diagnostics)
  - `AISRS-SDK-040` (catalog build with stable sort and hash)
  - `AISRS-SDK-041` (catalog index + pack/engine filters)
  - `AISRS-SDK-050` (compile plan skeleton into execution plan + workflow)
  - `AISRS-SDK-051` (compile workflow into execution plan with stable topological order + workflow preflight passthrough into plan meta)
  - `AISRS-SDK-052` (node readiness: missing refs / detect requirement / condition skipped)
  - `AISRS-SDK-053` (dry-run text/json with per-node report, issues, stable hashes)
  - Minor cleanup: simplified optional/object handling and chain-scope filtering code paths (`?`/`contains`) for clearer semantics.
- Planned next:
  - `AISRS-ENG-001+` engine events/loop integration
