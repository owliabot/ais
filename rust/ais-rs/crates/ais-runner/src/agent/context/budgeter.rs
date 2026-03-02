use super::super::budget::{compact_json_with_options, JsonBudgetOptions};
use super::budget_policy::{
    ContextCompactionPolicy, ContextPressureMode, PlannerContextStageLimits, ToolMemoryBudgetPolicy,
};
use super::collector::{prioritize_keys, registry_order_key, slot_order_key};
use ais_engine::EngineRunnerState;
use serde_json::{json, Value};

pub(in super::super) const DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET: usize = 6_000;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

#[derive(Debug, Clone, Copy)]
struct AdaptivePlannerBudget {
    base_token_limit: usize,
    effective_token_limit: usize,
    mode: &'static str,
    window_input_tokens: Option<u64>,
    remaining_tokens: Option<u64>,
    soft_limit_tokens: Option<u64>,
    usage_ratio_bps: Option<u64>,
    remaining_ratio_bps: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ContextCompactionStrategy {
    mode: ContextPressureMode,
    policy: ContextCompactionPolicy,
}

pub(in super::super) fn budget_and_compact_summary(
    base: Value,
    state: &EngineRunnerState,
    token_budget: usize,
) -> Value {
    let adaptive_budget = derive_adaptive_planner_budget(state, token_budget);
    let strategy = resolve_context_compaction_strategy(adaptive_budget);
    let mut budgeted = apply_planner_context_budget(base, adaptive_budget);
    apply_context_pressure_strategy(&mut budgeted, strategy);
    let mut compacted =
        compact_json_with_options(&budgeted, &strategy.policy.final_compact_options);
    refresh_context_budget_estimates(&mut compacted);
    compacted
}

fn derive_adaptive_planner_budget(
    state: &EngineRunnerState,
    base_token_limit: usize,
) -> AdaptivePlannerBudget {
    let usage = state.runtime.pointer("/agent/llm_usage");
    let window_input_tokens = usage
        .and_then(|value| value.get("context_window_input_tokens"))
        .and_then(Value::as_u64);
    let remaining_tokens = usage
        .and_then(|value| value.get("context_remaining_tokens"))
        .and_then(Value::as_u64);
    let soft_limit_tokens = usage
        .and_then(|value| value.get("context_soft_limit_tokens"))
        .and_then(Value::as_u64);
    let usage_ratio_from_window_bps = match (window_input_tokens, soft_limit_tokens) {
        (Some(window_input), Some(soft_limit)) if soft_limit > 0 => {
            Some(window_input.saturating_mul(10_000) / soft_limit)
        }
        _ => None,
    };
    let remaining_ratio_bps = match (remaining_tokens, soft_limit_tokens) {
        (Some(remaining), Some(soft_limit)) if soft_limit > 0 => {
            Some(remaining.saturating_mul(10_000) / soft_limit)
        }
        _ => None,
    };
    let usage_ratio_from_remaining_bps =
        usage_ratio_bps_from_remaining_tokens(remaining_tokens, soft_limit_tokens);
    let usage_ratio_bps = match (usage_ratio_from_window_bps, usage_ratio_from_remaining_bps) {
        (Some(window_ratio), Some(remaining_ratio)) => Some(window_ratio.max(remaining_ratio)),
        (Some(window_ratio), None) => Some(window_ratio),
        (None, Some(remaining_ratio)) => Some(remaining_ratio),
        (None, None) => None,
    };

    let (effective, mode) = ToolMemoryBudgetPolicy::derive_adaptive_effective_token_limit(
        base_token_limit,
        usage_ratio_bps,
    );

    AdaptivePlannerBudget {
        base_token_limit,
        effective_token_limit: effective,
        mode,
        window_input_tokens,
        remaining_tokens,
        soft_limit_tokens,
        usage_ratio_bps,
        remaining_ratio_bps,
    }
}

fn usage_ratio_bps_from_remaining_tokens(
    remaining_tokens: Option<u64>,
    soft_limit_tokens: Option<u64>,
) -> Option<u64> {
    match (remaining_tokens, soft_limit_tokens) {
        (Some(remaining), Some(soft_limit)) if soft_limit > 0 => Some(
            soft_limit
                .saturating_sub(remaining.min(soft_limit))
                .saturating_mul(10_000)
                / soft_limit,
        ),
        _ => None,
    }
}

fn resolve_context_compaction_strategy(budget: AdaptivePlannerBudget) -> ContextCompactionStrategy {
    let pressure_mode = ToolMemoryBudgetPolicy::derive_context_pressure_mode(
        budget.usage_ratio_bps,
        budget.remaining_tokens,
    );
    ContextCompactionStrategy {
        mode: pressure_mode,
        policy: ToolMemoryBudgetPolicy::context_compaction_policy(pressure_mode),
    }
}

fn apply_planner_context_budget(mut base: Value, budget: AdaptivePlannerBudget) -> Value {
    let token_budget = budget.effective_token_limit.max(1);
    let baseline_tokens = estimate_tokens_json(&base);
    let stages = ToolMemoryBudgetPolicy::planner_context_stages();
    let mut selected_stage = stages[stages.len() - 1];
    let mut selected_tokens = baseline_tokens;
    let mut selected = base.clone();
    let mut truncated = false;

    for stage in *stages {
        let mut candidate = base.clone();
        apply_context_stage(&mut candidate, stage);
        let estimated_tokens = estimate_tokens_json(&candidate);
        selected_stage = stage;
        selected_tokens = estimated_tokens;
        selected = candidate;
        if estimated_tokens <= token_budget as u64 {
            truncated = stage.name != "balanced" || estimated_tokens < baseline_tokens;
            break;
        }
    }

    if selected_tokens > token_budget as u64 {
        truncated = true;
    }
    if let Some(object) = selected.as_object_mut() {
        object.insert(
            "context_budget".to_string(),
            json!({
                "token_limit": token_budget,
                "base_token_limit": budget.base_token_limit,
                "adaptive_mode": budget.mode,
                "adaptive": {
                    "window_input_tokens": budget.window_input_tokens,
                    "remaining_tokens": budget.remaining_tokens,
                    "soft_limit_tokens": budget.soft_limit_tokens,
                    "usage_ratio_bps": budget.usage_ratio_bps,
                    "remaining_ratio_bps": budget.remaining_ratio_bps,
                },
                // Compatibility aliases: these existed before explicit payload/emitted scopes.
                "estimated_tokens": selected_tokens,
                "baseline_estimated_tokens": baseline_tokens,
                // Stage-selection budget scope: payload core before envelope/legacy metadata.
                "token_limit_scope": "payload_core",
                "estimated_payload_core_tokens": selected_tokens,
                "stage_selected_payload_core_tokens": selected_tokens,
                "baseline_payload_core_tokens": baseline_tokens,
                "estimator": "chars_div_4",
            "stage": selected_stage.name,
                "truncated": truncated,
            }),
        );
    }
    base = selected;
    base
}

fn refresh_context_budget_estimates(summary: &mut Value) {
    let token_limit = summary
        .pointer("/context_budget/token_limit")
        .and_then(Value::as_u64);
    let stage_selected_payload_core_tokens = summary
        .pointer("/context_budget/stage_selected_payload_core_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            summary
                .pointer("/context_budget/estimated_payload_core_tokens")
                .and_then(Value::as_u64)
        });
    let mut payload_core = summary.clone();
    if let Some(payload_object) = payload_core.as_object_mut() {
        payload_object.remove("context_budget");
    }
    let payload_core_tokens = estimate_tokens_json(&payload_core);

    if let Some(context_budget) = summary
        .pointer_mut("/context_budget")
        .and_then(Value::as_object_mut)
    {
        context_budget.insert(
            "token_limit_scope".to_string(),
            Value::String("payload_core".to_string()),
        );
        context_budget.insert(
            "estimated_tokens".to_string(),
            Value::Number(payload_core_tokens.into()),
        );
        context_budget.insert(
            "estimated_payload_core_tokens".to_string(),
            Value::Number(payload_core_tokens.into()),
        );
        context_budget.insert(
            "stage_selected_payload_core_tokens".to_string(),
            Value::Number(
                stage_selected_payload_core_tokens
                    .unwrap_or(payload_core_tokens)
                    .into(),
            ),
        );
    }

    let mut payload_tokens = estimate_tokens_json(summary);
    for _ in 0..3 {
        let payload_metadata_tokens = payload_tokens.saturating_sub(payload_core_tokens);
        if let Some(context_budget) = summary
            .pointer_mut("/context_budget")
            .and_then(Value::as_object_mut)
        {
            context_budget.insert(
                "estimated_payload_tokens".to_string(),
                Value::Number(payload_tokens.into()),
            );
            context_budget.insert(
                "estimated_payload_metadata_tokens".to_string(),
                Value::Number(payload_metadata_tokens.into()),
            );
            context_budget.insert(
                "payload_within_token_limit".to_string(),
                Value::Bool(token_limit.is_some_and(|limit| payload_tokens <= limit)),
            );
        }
        let refreshed = estimate_tokens_json(summary);
        if refreshed == payload_tokens {
            break;
        }
        payload_tokens = refreshed;
    }
}

fn apply_context_pressure_strategy(summary: &mut Value, strategy: ContextCompactionStrategy) {
    let mut actions = Vec::<String>::new();
    if strategy.policy.drop_input_slot_canonical_refs
        && set_pointer_value(summary, "/input_slots/canonical_refs", Value::Null)
    {
        actions.push("drop_input_slots_canonical_refs".to_string());
    }

    if strategy.policy.drop_capability_protocols {
        if set_pointer_value(summary, "/capability_view/protocols", Value::Array(vec![])) {
            actions.push("drop_capability_protocols".to_string());
        }
        let topic_count = summary
            .pointer("/capability_view/topics")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        if let Some(slot) = summary.pointer_mut("/capability_view/counts") {
            *slot = json!({
                "protocols": 0,
                "actions": 0,
                "queries": 0,
                "topics": topic_count,
            });
        }
    }

    if let Some(options) = strategy.policy.tool_memory_compact_options {
        if compact_pointer_value(summary, "/tool_memory_projection", options) {
            actions.push("compress_tool_memory_projection".to_string());
        }
    }

    if let Some(options) = strategy.policy.failed_finalize_compact_options {
        if compact_pointer_value(summary, "/previous_error/last_failed_finalize", options) {
            actions.push("compress_last_failed_finalize".to_string());
        }
    }

    if strategy.policy.drop_last_failed_assistant_content
        && set_pointer_value(
            summary,
            "/previous_error/last_failed_finalize/assistant_content",
            Value::Null,
        )
    {
        actions.push("drop_last_failed_assistant_content".to_string());
    }

    let Some(context_budget) = summary
        .pointer_mut("/context_budget")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    context_budget.insert(
        "pressure_mode".to_string(),
        Value::String(strategy.mode.as_str().to_string()),
    );
    context_budget.insert(
        "pressure_actions".to_string(),
        Value::Array(actions.into_iter().map(Value::String).collect::<Vec<_>>()),
    );
}

fn set_pointer_value(target: &mut Value, path: &str, value: Value) -> bool {
    let Some(slot) = target.pointer_mut(path) else {
        return false;
    };
    *slot = value;
    true
}

fn compact_pointer_value(target: &mut Value, path: &str, options: JsonBudgetOptions) -> bool {
    let Some(slot) = target.pointer_mut(path) else {
        return false;
    };
    let compacted = compact_json_with_options(slot, &options);
    *slot = compacted;
    true
}

fn apply_context_stage(summary: &mut Value, stage: PlannerContextStageLimits) {
    trim_input_store(summary, stage.max_fact_entries);
    trim_input_slots(
        summary,
        stage.max_input_slots_resolved,
        stage.max_input_slots_missing,
    );
    trim_input_registry(summary, stage.max_registry_entries, stage.max_known_refs);
    trim_canonical_context(summary, stage.max_canonical_per_group);
    trim_node_output_refs(
        summary,
        stage.max_node_output_entries,
        stage.max_node_output_refs,
    );
    trim_capability_view(summary, stage);
    trim_previous_error(summary, stage.max_previous_error_issues);
}

fn trim_input_store(summary: &mut Value, max_entries: usize) {
    let facts = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object)
        .cloned();
    let meta = summary
        .pointer("/input_store/meta")
        .and_then(Value::as_object)
        .cloned();
    let (Some(facts), Some(mut meta)) = (facts, meta) else {
        return;
    };
    let selected_keys = prioritize_keys(facts.keys().cloned().collect(), max_entries);
    let mut projected_facts = serde_json::Map::<String, Value>::new();
    let mut projected_meta = serde_json::Map::<String, Value>::new();
    for key in &selected_keys {
        if let Some(value) = facts.get(key.as_str()) {
            projected_facts.insert(key.clone(), value.clone());
        }
        if let Some(value) = meta.get(key.as_str()) {
            projected_meta.insert(key.clone(), value.clone());
        }
    }
    let existing_truncated = meta
        .remove("_truncated_entries")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let newly_truncated = facts.len().saturating_sub(selected_keys.len()) as u64;
    let truncated = existing_truncated.saturating_add(newly_truncated);
    if truncated > 0 {
        projected_meta.insert(
            "_truncated_entries".to_string(),
            Value::Number(truncated.into()),
        );
    }

    if let Some(slot) = summary.pointer_mut("/input_store/facts") {
        *slot = Value::Object(projected_facts);
    }
    if let Some(slot) = summary.pointer_mut("/input_store/meta") {
        *slot = Value::Object(projected_meta);
    }
}

fn trim_input_slots(summary: &mut Value, max_resolved: usize, max_missing: usize) {
    let resolved = summary
        .pointer("/input_slots/resolved")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let missing = summary
        .pointer("/input_slots/missing")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut resolved_sorted = resolved;
    resolved_sorted.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
    resolved_sorted.truncate(max_resolved.max(1));
    if let Some(slot) = summary.pointer_mut("/input_slots/resolved") {
        *slot = Value::Array(resolved_sorted);
    }

    let mut missing_sorted = missing;
    missing_sorted.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
    missing_sorted.truncate(max_missing.max(1));
    if let Some(slot) = summary.pointer_mut("/input_slots/missing") {
        *slot = Value::Array(missing_sorted);
    }
}

fn trim_input_registry(summary: &mut Value, max_entries: usize, max_known_refs: usize) {
    let entries = summary
        .pointer("/input_registry/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut sorted_entries = entries;
    sorted_entries.sort_by(|left, right| registry_order_key(left).cmp(&registry_order_key(right)));
    sorted_entries.truncate(max_entries.max(1));
    if let Some(slot) = summary.pointer_mut("/input_registry/entries") {
        *slot = Value::Array(sorted_entries.clone());
    }

    let mut known_refs = sorted_entries
        .iter()
        .filter_map(|entry| entry.get("ref").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    known_refs.sort();
    known_refs.dedup();
    known_refs.truncate(max_known_refs.max(1));
    if let Some(slot) = summary.pointer_mut("/input_registry/known_refs") {
        *slot = Value::Array(
            known_refs
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect::<Vec<_>>(),
        );
    }
    if let Some(slot) = summary.pointer_mut("/input_registry/counts") {
        let resolved = sorted_entries
            .iter()
            .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("resolved"))
            .count();
        let missing = sorted_entries
            .iter()
            .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("missing"))
            .count();
        *slot = json!({
            "known_refs": known_refs.len(),
            "resolved": resolved,
            "missing": missing,
        });
    }
}

fn trim_canonical_context(summary: &mut Value, max_per_group: usize) {
    for path in [
        "/canonical_context/chain_refs",
        "/canonical_context/account_refs",
        "/canonical_context/asset_refs",
        "/canonical_context/amount_refs",
    ] {
        let items = summary
            .pointer(path)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut sorted = items;
        sorted.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
        sorted.truncate(max_per_group.max(1));
        if let Some(slot) = summary.pointer_mut(path) {
            *slot = Value::Array(sorted);
        }
    }
}

fn trim_node_output_refs(summary: &mut Value, max_entries: usize, max_refs: usize) {
    let mut entries = summary
        .pointer("/node_output_refs/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    entries.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
    entries.truncate(max_entries.max(1));
    for entry in &mut entries {
        if let Some(refs) = entry.get_mut("refs").and_then(Value::as_array_mut) {
            refs.truncate(max_refs.max(1));
        }
    }
    if let Some(slot) = summary.pointer_mut("/node_output_refs/entries") {
        *slot = Value::Array(entries.clone());
    }
    let mut known_refs = entries
        .iter()
        .flat_map(|entry| {
            entry
                .get("refs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    known_refs.sort();
    known_refs.dedup();
    if let Some(slot) = summary.pointer_mut("/node_output_refs/known_refs") {
        *slot = Value::Array(
            known_refs
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect::<Vec<_>>(),
        );
    }
    let step_total = summary
        .pointer("/node_output_refs/counts/steps")
        .and_then(Value::as_u64)
        .unwrap_or(entries.len() as u64);
    if let Some(slot) = summary.pointer_mut("/node_output_refs/counts") {
        *slot = json!({
            "steps": step_total,
            "entries": entries.len(),
            "known_refs": known_refs.len(),
        });
    }
}

fn trim_capability_view(summary: &mut Value, stage: PlannerContextStageLimits) {
    let protocols = summary
        .pointer("/capability_view/protocols")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if protocols.is_empty() {
        return;
    }
    let mut sorted = protocols;
    sorted.sort_by(|left, right| {
        let left_name = left
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let right_name = right
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        left_name.cmp(right_name)
    });
    sorted.truncate(stage.max_capability_protocols.max(1));
    for protocol in &mut sorted {
        if let Some(actions) = protocol.get_mut("actions").and_then(Value::as_array_mut) {
            actions.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
            actions.truncate(stage.max_actions_per_protocol.max(1));
        }
        if let Some(queries) = protocol.get_mut("queries").and_then(Value::as_array_mut) {
            queries.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
            queries.truncate(stage.max_queries_per_protocol.max(1));
        }
        if let Some(required_inputs) = protocol
            .get_mut("required_inputs")
            .and_then(Value::as_array_mut)
        {
            required_inputs.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
            required_inputs.truncate(stage.max_required_inputs_per_protocol.max(1));
        }
        if let Some(topics) = protocol.get_mut("topics").and_then(Value::as_array_mut) {
            topics.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
            topics.truncate(stage.max_required_inputs_per_protocol.max(1));
        }
        if let Some(topic_cards) = protocol
            .get_mut("topic_cards")
            .and_then(Value::as_array_mut)
        {
            topic_cards.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
            topic_cards.truncate(stage.max_actions_per_protocol.max(1));
            for card in topic_cards {
                if let Some(actions) = card.get_mut("actions").and_then(Value::as_array_mut) {
                    actions.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
                    actions.truncate(stage.max_actions_per_protocol.max(1));
                }
                if let Some(queries) = card.get_mut("queries").and_then(Value::as_array_mut) {
                    queries.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
                    queries.truncate(stage.max_queries_per_protocol.max(1));
                }
                if let Some(required_inputs) = card
                    .get_mut("required_inputs")
                    .and_then(Value::as_array_mut)
                {
                    required_inputs
                        .sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
                    required_inputs.truncate(stage.max_required_inputs_per_protocol.max(1));
                }
                if let Some(chains) = card.get_mut("chains").and_then(Value::as_array_mut) {
                    chains.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
                    chains.truncate(stage.max_queries_per_protocol.max(1));
                }
            }
        }
    }

    if let Some(slot) = summary.pointer_mut("/capability_view/protocols") {
        *slot = Value::Array(sorted.clone());
    }
    if let Some(slot) = summary.pointer_mut("/capability_view/topics") {
        let mut topics = sorted
            .iter()
            .flat_map(|protocol| {
                protocol
                    .get("topics")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        topics.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
        topics.dedup();
        *slot = Value::Array(topics);
    }
    if let Some(slot) = summary.pointer_mut("/capability_view/counts") {
        let action_count = sorted
            .iter()
            .map(|protocol| {
                protocol
                    .get("actions")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
        let query_count = sorted
            .iter()
            .map(|protocol| {
                protocol
                    .get("queries")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
        let topic_count = sorted
            .iter()
            .map(|protocol| {
                protocol
                    .get("topics")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
        *slot = json!({
            "protocols": sorted.len(),
            "actions": action_count,
            "queries": query_count,
            "topics": topic_count,
        });
    }
}

fn trim_previous_error(summary: &mut Value, max_issues: usize) {
    for path in ["/previous_error/issues", "/previous_error/error/issues"] {
        let issues = summary
            .pointer(path)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if issues.is_empty() {
            continue;
        }
        let mut sorted = issues;
        sorted.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
        sorted.truncate(max_issues.max(1));
        if let Some(slot) = summary.pointer_mut(path) {
            *slot = Value::Array(sorted);
        }
    }
}

pub(in super::super) fn estimate_tokens_json(value: &Value) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let chars = encoded.chars().count();
    chars
        .saturating_add(CHARS_PER_TOKEN_ESTIMATE.saturating_sub(1))
        .checked_div(CHARS_PER_TOKEN_ESTIMATE)
        .unwrap_or(0) as u64
}
