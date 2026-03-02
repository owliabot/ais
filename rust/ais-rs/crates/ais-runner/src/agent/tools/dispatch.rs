use crate::error::RunnerError;
use ais_llm::ToolCall;
use serde_json::{json, Value};

use super::super::budget::{compact_json_for_llm, compact_json_with_options};
use super::super::candidates::{CandidateContext, CandidateSearchRequest};
use super::super::context::budget_policy::ToolMemoryBudgetPolicy;
use super::super::intent_segmented::{
    coerce_required_scalar_string, control_semantics_search_hint_payload,
    decode_grounding_tool_args, decode_plan_sketch_segment_arg, decode_segment_tool_args,
    decode_todo_tool_args, guide_get_payload, is_control_semantics_query, parse_grounding_draft,
    parse_segment_draft, parse_todo_draft, resolve_missing_facts_payload, BeginToolArgs,
    CandidateDetailArgs, CatalogSearchArgs, CheckSegmentArgs, GuideGetArgs, IntentGroundingDraft,
    ListCandidatesArgs, PlannerRoundPhase, ResolveMissingFactsArgs, SegmentCheckContext,
    SegmentDraft, SegmentPlanningSession, TodoDraft,
};
use super::super::planning_memory::PlanningMemory;
use super::super::sanitize::{sanitize_for_llm_payload, sanitize_for_llm_payload_with_limit};
use super::decode::normalize_tool_args_for_validation;
use super::guide::{guide_get_payload_contains_full_schema, guide_get_requires_full_schema};
use super::phase_policy::ensure_tool_allowed_for_phase;

#[derive(Debug)]
pub(crate) enum PlannerToolOutput {
    Begin(SegmentPlanningSession),
    SegmentDraft(SegmentDraft),
    TodoDraft(TodoDraft),
    IntentGrounding(IntentGroundingDraft),
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

#[cfg(test)]
pub(crate) fn decode_segmented_tool_call(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    decode_segmented_tool_call_with_memory(
        tool,
        finalize_tool,
        phase,
        candidate_context,
        None,
        None,
        None,
    )
}

pub(crate) fn decode_segmented_tool_call_with_memory(
    tool: &ToolCall,
    finalize_tool: &str,
    phase: PlannerRoundPhase,
    candidate_context: Option<&CandidateContext>,
    segment_check_context: Option<&SegmentCheckContext>,
    memory: Option<&mut PlanningMemory>,
    projection_budget_tokens: Option<usize>,
) -> Result<DecodedSegmentedToolCall, RunnerError> {
    ensure_tool_allowed_for_phase(tool.name.as_str(), phase)?;

    let normalized_args = normalize_tool_args_for_validation(tool.name.as_str(), &tool.arguments);
    let compact_profile = ToolMemoryBudgetPolicy::derive_tool_dispatch_compact_profile(
        projection_budget_tokens
            .unwrap_or(ToolMemoryBudgetPolicy::TOOL_MEMORY_PROJECTION_DEFAULT_TOKENS),
    );
    let cache_key = super::cache::tool_cache_key(tool.name.as_str(), &normalized_args.arguments);
    let require_guide_schema_full =
        guide_get_requires_full_schema(tool.name.as_str(), &normalized_args.arguments);
    if let (Some(memory), Some(cache_key)) = (memory.as_ref(), cache_key.as_ref()) {
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

    match tool.name.as_str() {
        "list_candidates" => {
            let args: ListCandidatesArgs =
                serde_json::from_value(tool.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid list_candidates args: {error}"))
                })?;
            let content =
                serde_json::to_string(&super::super::intent_segmented::candidate_snapshot(
                    candidate_context,
                    Some(args.normalized_filter()),
                ))
                .map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "list_candidates".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "get_candidate_detail" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "candidate detail tool is unavailable".to_string(),
                ));
            };
            let args: CandidateDetailArgs =
                serde_json::from_value(normalized_args.arguments.clone()).map_err(|error| {
                    RunnerError::Llm(format!("invalid get_candidate_detail args: {error}"))
                })?;
            let details = context.get_details_for_refs(&args.refs);
            let sanitized = sanitize_for_llm_payload(&details);
            let compacted = compact_json_with_options(
                &sanitized,
                &ToolMemoryBudgetPolicy::tool_dispatch_candidate_detail_options(compact_profile),
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "get_candidate_detail".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
        }
        "catalog.search" => {
            let Some(context) = candidate_context else {
                return Err(RunnerError::Llm(
                    "catalog search tool is unavailable".to_string(),
                ));
            };
            let args: CatalogSearchArgs = serde_json::from_value(normalized_args.arguments.clone())
                .map_err(|error| {
                    RunnerError::Llm(format!("invalid catalog.search args: {error}"))
                })?;
            let query = args.query;
            let searched = if is_control_semantics_query(query.as_deref()) {
                control_semantics_search_hint_payload(
                    query.clone(),
                    args.kind.clone(),
                    args.chain.clone(),
                    args.min_risk_level,
                    args.max_risk_level,
                    args.limit,
                )
            } else {
                context.search_candidates(&CandidateSearchRequest {
                    query,
                    kind: args.kind,
                    chain: args.chain,
                    min_risk_level: args.min_risk_level,
                    max_risk_level: args.max_risk_level,
                    limit: args.limit,
                })
            };
            let sanitized = sanitize_for_llm_payload(&searched);
            let compacted = compact_json_for_llm(&sanitized);
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "catalog.search".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
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
            let sanitized = sanitize_for_llm_payload(&payload);
            let compacted = compact_json_with_options(
                &sanitized,
                &ToolMemoryBudgetPolicy::tool_dispatch_missing_facts_options(compact_profile),
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "catalog.resolve_missing_facts".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
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
            let compacted = if is_schema_request {
                if schema_has_full_json {
                    compact_json_with_options(
                        &sanitized,
                        &ToolMemoryBudgetPolicy::tool_dispatch_guide_schema_full_options(
                            compact_profile,
                        ),
                    )
                } else {
                    compact_json_with_options(
                        &sanitized,
                        &ToolMemoryBudgetPolicy::tool_dispatch_guide_schema_digest_options(
                            compact_profile,
                        ),
                    )
                }
            } else {
                compact_json_with_options(
                    &sanitized,
                    &ToolMemoryBudgetPolicy::tool_dispatch_guide_topic_options(compact_profile),
                )
            };
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
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
            let Some(check_context) = segment_check_context else {
                return Err(RunnerError::Llm(
                    "plan.check_segment is unavailable before plan.begin".to_string(),
                ));
            };
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
                Ok(segment) => match super::super::compile_segment_plan_with_snapshot_hash(
                    check_context.intent.as_str(),
                    check_context.session_id.as_str(),
                    check_context.cursor.as_str(),
                    &segment,
                    context,
                    check_context.pack_snapshot_hash.as_str(),
                    check_context.chain_scope.as_slice(),
                ) {
                    Ok(plan) => json!({
                        "ok": true,
                        "segment_id": segment.segment_id,
                        "node_count": plan.nodes.len(),
                        "issues": []
                    }),
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
            let sanitized = sanitize_for_llm_payload(&payload);
            let compacted = compact_json_with_options(
                &sanitized,
                &ToolMemoryBudgetPolicy::tool_dispatch_check_segment_options(compact_profile),
            );
            let content = serde_json::to_string(&compacted).map_err(RunnerError::from)?;
            if let (Some(memory), Some(cache_key)) = (memory, cache_key) {
                memory.insert(cache_key, content.clone());
            }
            Ok(DecodedSegmentedToolCall::ToolMessage {
                tool_name: "plan.check_segment".to_string(),
                tool_call_id: tool.id.clone(),
                content,
                cached: false,
            })
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
        other => Err(RunnerError::Llm(format!(
            "unsupported segmented planner tool `{other}`"
        ))),
    }
}
