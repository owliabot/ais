use crate::error::RunnerError;
use ais_llm::ToolCall;
use serde_json::{json, Value};
use std::collections::BTreeSet;

use super::super::budget::{compact_json_for_llm, compact_json_with_options};
use super::super::candidates::CandidateContext;
use super::super::context::budget_policy::{
    ToolDispatchCompactProfile, ToolDispatchKind, ToolMemoryBudgetPolicy,
};
use super::super::context::packing::ContextCompressLevel;
use super::super::input_store::InputStore;
use super::super::intent_segmented::{
    coerce_required_scalar_string, decode_grounding_tool_args, decode_plan_sketch_segment_arg,
    decode_segment_tool_args, decode_todo_tool_args, guide_get_payload, parse_grounding_draft,
    parse_segment_draft, parse_todo_draft, resolve_missing_facts_payload, IntentGroundingDraft,
    PlannerRoundPhase, SegmentCheckContext, SegmentDraft, SegmentPlanningSession, TodoDraft,
};
use super::super::planning_memory::PlanningMemory;
use super::super::runtime_facts_store::RuntimeFactsStore;
use super::super::sanitize::{sanitize_for_llm_payload, sanitize_for_llm_payload_with_limit};
use super::super::state_summary::StateSummary;
use super::args::{
    AbortIntentArgs, BeginToolArgs, CheckSegmentArgs, GuideGetArgs, ResolveMissingFactsArgs,
};
use super::catalog::{self, CatalogDiscoverPayload};
use super::decode::normalize_tool_args_for_validation;
use super::guide::{guide_get_payload_contains_full_schema, guide_get_requires_full_schema};
use super::phase_policy::ensure_tool_allowed_for_phase;
use super::runtime_query::{self, RuntimeQueryArgs};

#[derive(Debug)]
pub(crate) enum PlannerToolOutput {
    Begin(SegmentPlanningSession),
    SegmentDraft(SegmentDraft),
    TodoDraft(TodoDraft),
    IntentGrounding(IntentGroundingDraft),
    AbortIntent(AbortIntentOutput),
}

#[derive(Debug, Clone)]
pub(crate) struct AbortIntentOutput {
    pub(crate) reason_code: String,
    pub(crate) summary: String,
    pub(crate) evidence: Value,
    pub(crate) user_fix_hint: Option<String>,
}

#[derive(Debug)]
pub(crate) enum DecodedSegmentedToolCall {
    Final(PlannerToolOutput),
    ToolMessage {
        tool_name: String,
        tool_call_id: String,
        content: String,
        cached: bool,
    },
}

// ── shared readonly dispatch pipeline ──────────────────────────────────
//
// The pattern "sanitize → compact → serialize → cache → ToolMessage" is
// repeated for every readonly tool.  The helpers below collapse that into
// a single call-site so that adding a new readonly tool only needs to
// produce a `Value` payload and pick a `ToolDispatchKind`.

/// Resolve compact profile from the caller-supplied compress-level /
/// projection-budget pair.
fn resolve_compact_profile(
    compress_level: Option<ContextCompressLevel>,
    projection_budget_tokens: Option<usize>,
) -> ToolDispatchCompactProfile {
    match compress_level {
        Some(level) => {
            ToolMemoryBudgetPolicy::derive_tool_dispatch_compact_profile_from_compress_level(level)
        }
        None => ToolMemoryBudgetPolicy::derive_tool_dispatch_compact_profile(
            projection_budget_tokens
                .unwrap_or(ToolMemoryBudgetPolicy::tool_memory_projection_default_tokens()),
        ),
    }
}

/// Standard pipeline: sanitize → compact(kind, profile) → serialize → cache → ToolMessage
fn readonly_tool_message(
    tool_name: &str,
    tool_call_id: &str,
    payload: &Value,
    kind: ToolDispatchKind,
    compact_profile: ToolDispatchCompactProfile,
    memory: Option<&mut PlanningMemory>,
    cache_key: Option<String>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    let sanitized = sanitize_for_llm_payload(payload);
    let compacted = compact_json_with_options(
        &sanitized,
        &ToolMemoryBudgetPolicy::tool_dispatch_options(kind, compact_profile),
    );
    let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
    if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
        memory.insert(cache_key, content.clone());
    }
    Ok(DecodedSegmentedToolCall::ToolMessage {
        tool_name: tool_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        content,
        cached: false,
    })
}

/// Variant using `compact_json_for_llm` (default budget) instead of
/// kind-specific options.
fn readonly_tool_message_default_compact(
    tool_name: &str,
    tool_call_id: &str,
    payload: &Value,
    memory: Option<&mut PlanningMemory>,
    cache_key: Option<String>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    let sanitized = sanitize_for_llm_payload(payload);
    let compacted = compact_json_for_llm(&sanitized);
    let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
    if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
        memory.insert(cache_key, content.clone());
    }
    Ok(DecodedSegmentedToolCall::ToolMessage {
        tool_name: tool_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        content,
        cached: false,
    })
}

/// Bundles the shared context needed by all tool dispatch calls.
pub(crate) struct ToolDispatchContext<'a> {
    pub(crate) finalize_tool: &'a str,
    pub(crate) phase: PlannerRoundPhase,
    pub(crate) candidate_context: Option<&'a CandidateContext>,
    pub(crate) segment_check_context: Option<&'a SegmentCheckContext>,
    pub(crate) memory: Option<&'a mut PlanningMemory>,
    pub(crate) projection_budget_tokens: Option<usize>,
    pub(crate) compress_level: Option<ContextCompressLevel>,
    pub(crate) typed_summary: Option<&'a StateSummary>,
    pub(crate) runtime_facts_store: Option<&'a RuntimeFactsStore>,
    pub(crate) input_store: Option<&'a InputStore>,
}

#[cfg(test)]
pub(crate) fn decode_segmented_tool_call(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    let mut ctx = ToolDispatchContext {
        finalize_tool,
        phase,
        candidate_context,
        segment_check_context: None,
        memory: None,
        projection_budget_tokens: None,
        compress_level: None,
        typed_summary: None,
        runtime_facts_store: None,
        input_store: None,
    };
    decode_segmented_tool_call_impl(tool, &mut ctx)
}

pub(crate) fn decode_segmented_tool_call_with_memory(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
    segment_check_context: Option<&SegmentCheckContext>,
    memory: Option<&mut PlanningMemory>,
    projection_budget_tokens: Option<usize>,
    compress_level: Option<ContextCompressLevel>,
    _packed_summary: Option<&Value>,
    typed_summary: Option<&StateSummary>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    let mut ctx = ToolDispatchContext {
        finalize_tool,
        phase,
        candidate_context,
        segment_check_context,
        memory,
        projection_budget_tokens,
        compress_level,
        typed_summary,
        runtime_facts_store: None,
        input_store: None,
    };
    decode_segmented_tool_call_impl(tool, &mut ctx)
}

fn collect_allowed_recovery_attempt_keys(typed_summary: Option<&StateSummary>) -> BTreeSet<String> {
    let mut out = BTreeSet::<String>::new();
    if let Some(summary) = typed_summary {
        for key in summary.allowed_recovery_attempt_keys() {
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
fn collect_allowed_recovery_attempt_keys_from_summary(
    typed_summary: Option<&StateSummary>,
    summary: Option<&Value>,
) -> BTreeSet<String> {
    let mut out = collect_allowed_recovery_attempt_keys(typed_summary);
    if let Some(keys) = summary
        .and_then(|value| value.pointer("/recovery_diagnostics/available_attempt_keys"))
        .and_then(Value::as_array)
    {
        for key in keys.iter().filter_map(Value::as_str) {
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_string());
            }
        }
    }
    if let Some(keys) = summary
        .and_then(|value| value.pointer("/previous_error/autofill_history/attempt_keys"))
        .and_then(Value::as_array)
    {
        for key in keys.iter().filter_map(Value::as_str) {
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state_summary::InputBindingContract;

    #[test]
    fn collect_allowed_recovery_attempt_keys_prefers_typed_views() {
        let typed_summary = StateSummary {
            completed_segments: 0,
            completed_nodes: 0,
            plan_epoch: 0,
            paused_reason: None,
            done: false,
            previous_error: Some(json!({
                "autofill_history": {
                    "attempt_keys": ["history.retry"]
                }
            })),
            input_store: None,
            runtime_facts: None,
            input_binding: InputBindingContract {
                schema: "ais-agent-input-binding-contract/0.0.1",
                bindable_namespace: "inputs",
                bindable_refs_source: "state_summary.input_store",
                bindable_refs_projection: "state_summary.input_registry.known_refs",
                known_refs_only: true,
                facts_bindable: false,
            },
            input_registry: json!({"known_refs":[]}),
            node_output_refs: json!({"known_refs":[]}),
            reusable_outputs: None,
            tool_memory_projection: None,
            intent_slots: None,
            intent_context: None,
            capability_view: None,
            capability_ready: None,
            side_effect_lifecycle: None,
            todo_state: None,
            recovery_diagnostics: Some(json!({
                "available_attempt_keys": ["runtime.query.resolve"]
            })),
        };
        let raw_summary = json!({
            "recovery_diagnostics": {
                "available_attempt_keys": ["raw.only"]
            }
        });

        let keys = collect_allowed_recovery_attempt_keys_from_summary(
            Some(&typed_summary),
            Some(&raw_summary),
        );

        assert!(keys.contains("runtime.query.resolve"));
        assert!(keys.contains("history.retry"));
        assert!(keys.contains("raw.only"));
    }
}

#[allow(dead_code)]
pub(crate) fn decode_segmented_tool_call_full(
    tool: &ToolCall,
    ctx: &mut ToolDispatchContext<'_>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    decode_segmented_tool_call_impl(tool, ctx)
}

fn decode_segmented_tool_call_impl(
    tool: &ToolCall,
    ctx: &mut ToolDispatchContext<'_>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    ensure_tool_allowed_for_phase(tool.name.as_str(), ctx.phase)?;

    let normalized_args = normalize_tool_args_for_validation(tool.name.as_str(), &tool.arguments);
    let compact_profile = resolve_compact_profile(ctx.compress_level, ctx.projection_budget_tokens);
    let cache_key = super::cache::tool_cache_key(tool.name.as_str(), &normalized_args.arguments);
    let require_guide_schema_full =
        guide_get_requires_full_schema(tool.name.as_str(), &normalized_args.arguments);
    if let (Some(memory), Some(cache_key)) = (ctx.memory.as_ref(), cache_key.as_ref()) {
        if let Some(content) = memory.get(cache_key.as_str()) {
            let can_use_cached =
                !require_guide_schema_full || guide_get_payload_contains_full_schema(content);
            if can_use_cached {
                return Ok(DecodedSegmentedToolCall::ToolMessage {
                    tool_name: tool.name.clone(),
                    tool_call_id: tool.id.clone(),
                    content: content.to_string(),
                    cached: true,
                });
            }
        }
    }

    let candidate_context = ctx.candidate_context;
    let finalize_tool = ctx.finalize_tool;

    match tool.name.as_str() {
        "get_candidate_detail" => {
            let payload = catalog::decode_candidate_detail_payload(
                normalized_args.arguments.clone(),
                candidate_context,
            )?;
            readonly_tool_message(
                "get_candidate_detail",
                &tool.id,
                &payload,
                ToolDispatchKind::CandidateDetail,
                compact_profile,
                ctx.memory.take(),
                cache_key,
            )
        }
        "catalog.discover" => {
            match catalog::decode_catalog_discover_payload(
                normalized_args.arguments.clone(),
                candidate_context,
            )? {
                CatalogDiscoverPayload::Search(payload) => readonly_tool_message_default_compact(
                    "catalog.discover",
                    &tool.id,
                    &payload,
                    ctx.memory.take(),
                    cache_key,
                ),
                CatalogDiscoverPayload::Inventory(payload) => {
                    let content = serde_json::to_string(&payload).map_err(RunnerError::from)?;
                    if let (Some(memory), Some(cache_key)) = (ctx.memory.as_deref_mut(), cache_key)
                    {
                        memory.insert(cache_key, content.clone());
                    }
                    Ok(DecodedSegmentedToolCall::ToolMessage {
                        tool_name: "catalog.discover".to_string(),
                        tool_call_id: tool.id.clone(),
                        content,
                        cached: false,
                    })
                }
            }
        }
        "catalog.resolve_missing_facts" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "catalog.resolve_missing_facts requires workspace candidate context"
                        .to_string(),
                ));
            };
            let args: ResolveMissingFactsArgs =
                serde_json::from_value(normalized_args.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!(
                        "invalid catalog.resolve_missing_facts args: {error}"
                    ))
                })?;
            let payload = resolve_missing_facts_payload(context, &args);
            readonly_tool_message(
                "catalog.resolve_missing_facts",
                &tool.id,
                &payload,
                ToolDispatchKind::MissingFacts,
                compact_profile,
                ctx.memory.take(),
                cache_key,
            )
        }
        "guide.get" => {
            let args: GuideGetArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| RunnerError::Llm(format!("invalid guide.get args: {error}")))?;
            let payload = guide_get_payload(args);
            let is_schema_request = payload.get("kind").and_then(Value::as_str) == Some("schema");
            let schema_has_full_json = payload.pointer("/schema/json").is_some();
            let sanitized = if is_schema_request && schema_has_full_json {
                sanitize_for_llm_payload_with_limit(&payload, 16_000)
            } else {
                sanitize_for_llm_payload(&payload)
            };
            let kind = if is_schema_request {
                if schema_has_full_json {
                    ToolDispatchKind::GuideSchemaFull
                } else {
                    ToolDispatchKind::GuideSchemaDigest
                }
            } else {
                ToolDispatchKind::GuideTopic
            };
            let compacted = compact_json_with_options(
                &sanitized,
                &ToolMemoryBudgetPolicy::tool_dispatch_options(kind, compact_profile),
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (ctx.memory.as_deref_mut(), cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "guide.get".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "plan.check_segment" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "plan.check_segment requires workspace candidate context".to_string(),
                ));
            };
            let Some(check_context) = ctx.segment_check_context else {
                return Err(RunnerError::Llm(
                    "plan.check_segment is unavailable before plan.begin".to_string(),
                ));
            };

            // `plan.check_segment` must validate against the same snapshot the planner sees.
            // In the segmented planner flow we pass the snapshot as `typed_summary`, so when the
            // host doesn't have direct store handles here we rehydrate them from projections.
            let derived_input_store = if ctx.input_store.is_some() {
                None
            } else {
                ctx.typed_summary
                    .and_then(|summary| summary.input_store.as_ref())
                    .and_then(InputStore::from_projected_planning_value)
            };
            let derived_runtime_facts_store = if ctx.runtime_facts_store.is_some() {
                None
            } else {
                ctx.typed_summary
                    .and_then(|summary| summary.runtime_facts.as_ref())
                    .and_then(RuntimeFactsStore::from_projected_planning_value)
            };
            let input_store_ref = ctx.input_store.or(derived_input_store.as_ref());
            let runtime_facts_store_ref = ctx
                .runtime_facts_store
                .or(derived_runtime_facts_store.as_ref());

            let args: CheckSegmentArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid plan.check_segment args: {error}"))
                })?;
            let segment = decode_plan_sketch_segment_arg(&args.segment)?;
            let payload = match super::super::canonicalize_segment_input_refs(
                &segment,
                &check_context.known_input_refs,
                &check_context.grounding_fact_keys,
            ) {
                Ok(segment) => match super::super::compile_segment_plan_with_snapshot_hash_and_policy(
                    check_context.intent.as_str(),
                    check_context.session_id.as_str(),
                    check_context.cursor.as_str(),
                    &segment,
                    context,
                    check_context.pack_snapshot_hash.as_str(),
                    check_context.chain_scope.as_slice(),
                    check_context.known_input_refs.as_slice(),
                    check_context.volatile_facts_policy,
                    runtime_facts_store_ref,
                    input_store_ref,
                ) {
                    Ok(plan) => {
                        match super::super::validate_segment_todo_scope_with_runtime_facts_and_policy(
                            &segment,
                            context,
                            check_context.current_todo_scope.as_deref(),
                            ctx.typed_summary,
                            runtime_facts_store_ref,
                            input_store_ref,
                            check_context.volatile_facts_policy,
                        ) {
                            Ok(()) => json!({
                                "ok": true,
                                "segment_id": segment.segment_id,
                                "node_count": plan.nodes.len(),
                                "issues": []
                            }),
                            Err(error) => json!({
                                "ok": false,
                                "segment_id": segment.segment_id,
                                "reason_code": error.get("reason_code").cloned().unwrap_or_else(|| json!("todo_scope_violation")),
                                "issues": error.get("issues").cloned().unwrap_or_else(|| Value::Array(vec![])),
                                "error": error
                            }),
                        }
                    }
                    Err(error) => json!({
                        "ok": false,
                        "segment_id": segment.segment_id,
                        "reason_code": error.get("reason_code").cloned().unwrap_or_else(|| json!("compile_error")),
                        "issues": error.get("issues").cloned().unwrap_or_else(|| Value::Array(vec![])),
                        "error": error
                    }),
                },
                Err(error) => json!({
                    "ok": false,
                    "segment_id": segment.segment_id,
                    "reason_code": error.get("reason_code").cloned().unwrap_or_else(|| json!("compile_error")),
                    "issues": error.get("issues").cloned().unwrap_or_else(|| Value::Array(vec![])),
                    "error": error
                }),
            };
            readonly_tool_message(
                "plan.check_segment",
                &tool.id,
                &payload,
                ToolDispatchKind::CheckSegment,
                compact_profile,
                ctx.memory.take(),
                cache_key,
            )
        }
        "plan.begin" => {
            if finalize_tool != "plan.begin" {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args: BeginToolArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| RunnerError::Llm(format!("invalid plan.begin args: {error}")))?;
            let session_id = coerce_required_scalar_string("session_id", &args.session_id)?;
            let snapshot_hash =
                coerce_required_scalar_string("snapshot_hash", &args.snapshot_hash)?;
            let cursor = coerce_required_scalar_string("cursor", &args.cursor)?;
            Ok(DecodedSegmentedToolCall::Final(PlannerToolOutput::Begin(
                SegmentPlanningSession {
                    session_id,
                    snapshot_hash,
                    cursor,
                    max_rounds: args.limits.max_rounds.max(1),
                    max_segments: args.limits.max_segments.max(1),
                },
            )))
        }
        "plan.ground_intent" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args =
                decode_grounding_tool_args(normalized_args.arguments.clone(), tool.name.as_str())?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::IntentGrounding(parse_grounding_draft(args)?),
            ))
        }
        "plan.propose_todos" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args =
                decode_todo_tool_args(normalized_args.arguments.clone(), tool.name.as_str())?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::TodoDraft(parse_todo_draft(args)?),
            ))
        }
        "plan.propose_segment" | "plan.revise_segment" => {
            if tool.name != finalize_tool {
                return Err(RunnerError::Llm(format!(
                    "planner called `{}` while expecting `{}`",
                    tool.name, finalize_tool
                )));
            }
            let args =
                decode_segment_tool_args(normalized_args.arguments.clone(), tool.name.as_str())?;
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::SegmentDraft(parse_segment_draft(args)?),
            ))
        }
        "plan.abort_intent" => {
            let args: AbortIntentArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| {
                RunnerError::Llm(format!("invalid plan.abort_intent args: {error}"))
            })?;
            let reason_code = args.reason_code.trim().to_string();
            if reason_code.is_empty() {
                return Err(RunnerError::Llm(
                    "plan.abort_intent requires non-empty reason_code".to_string(),
                ));
            }
            let summary = args.summary.trim().to_string();
            if summary.len() < 10 {
                return Err(RunnerError::Llm(
                    "plan.abort_intent requires summary with at least 10 characters".to_string(),
                ));
            }
            if args.evidence.attempted_recovery.is_empty() {
                return Err(RunnerError::Llm(
                    "plan.abort_intent requires non-empty evidence.attempted_recovery".to_string(),
                ));
            }
            let requested_attempts = args
                .evidence
                .attempted_recovery
                .iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<BTreeSet<_>>();
            if requested_attempts.is_empty() {
                return Err(RunnerError::Llm(
                    "plan.abort_intent requires non-empty evidence.attempted_recovery".to_string(),
                ));
            }
            let allowed_attempts = collect_allowed_recovery_attempt_keys(ctx.typed_summary);
            if allowed_attempts.is_empty() {
                return Err(RunnerError::Llm(
                    "plan.abort_intent rejected: host recovery history is unavailable".to_string(),
                ));
            }
            if let Some(unknown_attempt) = requested_attempts
                .iter()
                .find(|attempt| !allowed_attempts.contains(attempt.as_str()))
            {
                return Err(RunnerError::Llm(format!(
                    "plan.abort_intent rejected: attempted_recovery contains unknown entry `{unknown_attempt}`"
                )));
            }
            if let Some(context) = candidate_context {
                let canonical_missing_refs = args
                    .evidence
                    .missing_refs
                    .iter()
                    .filter_map(|reference| {
                        super::super::input_normalize::canonical_missing_ref(reference)
                    })
                    .collect::<Vec<_>>();
                if !canonical_missing_refs.is_empty() {
                    let recoverable =
                        super::super::intent_segmented::resolve_missing_facts_for_refs(
                            context,
                            canonical_missing_refs.as_slice(),
                            1,
                        )
                        .pointer("/resolved")
                        .and_then(Value::as_array)
                        .is_some_and(|items| !items.is_empty());
                    if recoverable {
                        return Err(RunnerError::Llm(
                            "plan.abort_intent rejected: unresolved refs still have recoverable query candidates"
                                .to_string(),
                        ));
                    }
                }
            }
            let evidence = json!({
                "attempted_recovery": args.evidence.attempted_recovery,
                "invalid_fields": args.evidence.invalid_fields,
                "missing_refs": args.evidence.missing_refs,
            });
            Ok(DecodedSegmentedToolCall::Final(
                PlannerToolOutput::AbortIntent(AbortIntentOutput {
                    reason_code,
                    summary,
                    evidence,
                    user_fix_hint: args
                        .user_fix_hint
                        .map(|hint| hint.trim().to_string())
                        .filter(|hint| !hint.is_empty()),
                }),
            ))
        }
        "runtime.query" => {
            let args: RuntimeQueryArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid runtime.query args: {error}"))
                })?;
            match args.action.as_str() {
                "inspect" => {
                    let payload = runtime_query::handle_inspect(
                        &args,
                        ctx.typed_summary,
                        ctx.runtime_facts_store,
                        ctx.input_store,
                    );
                    readonly_tool_message(
                        "runtime.query",
                        &tool.id,
                        &payload,
                        ToolDispatchKind::GuideTopic,
                        compact_profile,
                        ctx.memory.take(),
                        cache_key,
                    )
                }
                "resolve" => {
                    let payload = runtime_query::handle_resolve(
                        &args,
                        ctx.typed_summary,
                        ctx.runtime_facts_store,
                        ctx.input_store,
                        candidate_context,
                    );
                    readonly_tool_message(
                        "runtime.query",
                        &tool.id,
                        &payload,
                        ToolDispatchKind::MissingFacts,
                        compact_profile,
                        ctx.memory.take(),
                        cache_key,
                    )
                }
                other => Err(RunnerError::Llm(format!(
                    "unsupported runtime.query action `{other}`; expected: inspect, resolve"
                ))),
            }
        }
        other => Err(RunnerError::Llm(format!(
            "unsupported segmented planner tool `{other}`"
        ))),
    }
}
