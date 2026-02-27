use super::budget::{compact_json_for_llm, compact_json_with_options, JsonBudgetOptions};
use super::facts::FactStore;
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_engine::EngineRunnerState;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_FACT_ENTRIES_IN_SUMMARY: usize = 24;
pub(super) const DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET: usize = 6_000;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const CONTEXT_USAGE_LIGHT_THRESHOLD_BPS: u64 = 7_000;
const CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS: u64 = 8_500;
const CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS: u64 = 9_200;
const CONTEXT_PRESSURE_TIGHT_REMAINING_TOKENS: u64 = 8_000;
const CONTEXT_PRESSURE_CRITICAL_REMAINING_TOKENS: u64 = 3_000;
const ADAPTIVE_RELAXED_MAX_MULTIPLIER: usize = 3;
const ADAPTIVE_MEDIUM_NUMERATOR: usize = 17;
const ADAPTIVE_MEDIUM_DENOMINATOR: usize = 20;
const ADAPTIVE_CRITICAL_NUMERATOR: usize = 3;
const ADAPTIVE_CRITICAL_DENOMINATOR: usize = 5;
const PRIORITY_SLOTS: &[&str] = &[
    "owner",
    "wallet.default",
    "chain",
    "chain_id",
    "chain_ref",
    "network",
    "recipient",
    "to",
    "from",
    "sender",
    "token",
    "token.address",
    "asset",
    "asset.address",
    "amount",
    "amount.human",
    "amount.atomic",
];

#[derive(Debug, Clone, Copy)]
struct PlannerContextStage {
    name: &'static str,
    max_fact_entries: usize,
    max_input_slots_resolved: usize,
    max_input_slots_missing: usize,
    max_registry_entries: usize,
    max_known_refs: usize,
    max_canonical_per_group: usize,
    max_capability_protocols: usize,
    max_actions_per_protocol: usize,
    max_queries_per_protocol: usize,
    max_required_inputs_per_protocol: usize,
    max_previous_error_issues: usize,
}

const PLANNER_CONTEXT_STAGES: [PlannerContextStage; 3] = [
    PlannerContextStage {
        name: "balanced",
        max_fact_entries: 24,
        max_input_slots_resolved: 48,
        max_input_slots_missing: 32,
        max_registry_entries: 64,
        max_known_refs: 96,
        max_canonical_per_group: 32,
        max_capability_protocols: 24,
        max_actions_per_protocol: 16,
        max_queries_per_protocol: 16,
        max_required_inputs_per_protocol: 24,
        max_previous_error_issues: 12,
    },
    PlannerContextStage {
        name: "tight",
        max_fact_entries: 16,
        max_input_slots_resolved: 24,
        max_input_slots_missing: 16,
        max_registry_entries: 32,
        max_known_refs: 48,
        max_canonical_per_group: 16,
        max_capability_protocols: 12,
        max_actions_per_protocol: 8,
        max_queries_per_protocol: 8,
        max_required_inputs_per_protocol: 12,
        max_previous_error_issues: 8,
    },
    PlannerContextStage {
        name: "minimal",
        max_fact_entries: 8,
        max_input_slots_resolved: 12,
        max_input_slots_missing: 8,
        max_registry_entries: 16,
        max_known_refs: 24,
        max_canonical_per_group: 8,
        max_capability_protocols: 6,
        max_actions_per_protocol: 4,
        max_queries_per_protocol: 4,
        max_required_inputs_per_protocol: 6,
        max_previous_error_issues: 4,
    },
];

#[derive(Debug, Clone)]
pub(super) struct PlanningContextManager {
    last_hash: Option<String>,
    version: u64,
    token_budget: usize,
}

impl PlanningContextManager {
    pub(super) fn with_token_budget(token_budget: usize) -> Self {
        Self {
            last_hash: None,
            version: 0,
            token_budget: token_budget.max(1),
        }
    }

    pub(super) fn next_summary(
        &mut self,
        state: &EngineRunnerState,
        completed_segments: usize,
        done: bool,
        previous_error: Option<&Value>,
        fact_store: Option<&FactStore>,
        tool_memory_projection: Option<&Value>,
    ) -> Value {
        let base = build_projected_summary_with_budget(
            state,
            completed_segments,
            done,
            previous_error,
            fact_store,
            tool_memory_projection,
            self.token_budget,
        );
        let hash = stable_hash_hex(&base, &StableJsonOptions::default())
            .unwrap_or_else(|_| "context-hash-unavailable".to_string());
        let unchanged = self.last_hash.as_deref() == Some(hash.as_str());
        self.version = self.version.saturating_add(1);
        self.last_hash = Some(hash.clone());

        let mut object = base.as_object().cloned().unwrap_or_default();
        object.insert(
            "context_version".to_string(),
            Value::Number(self.version.into()),
        );
        object.insert("context_hash".to_string(), Value::String(hash));
        object.insert("context_unchanged".to_string(), Value::Bool(unchanged));
        let wrapped = Value::Object(object);
        compact_json_for_llm(&wrapped)
    }
}

impl Default for PlanningContextManager {
    fn default() -> Self {
        Self::with_token_budget(DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET)
    }
}

#[allow(dead_code)]
pub(super) fn build_projected_summary(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    fact_store: Option<&FactStore>,
    tool_memory_projection: Option<&Value>,
) -> Value {
    build_projected_summary_with_budget(
        state,
        completed_segments,
        done,
        previous_error,
        fact_store,
        tool_memory_projection,
        DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET,
    )
}

fn build_projected_summary_with_budget(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    fact_store: Option<&FactStore>,
    tool_memory_projection: Option<&Value>,
    token_budget: usize,
) -> Value {
    let input_slots = build_input_slots_projection(state, fact_store);
    let base = json!({
        "completed_segments": completed_segments,
        "completed_nodes": state.completed_node_ids.len(),
        "plan_epoch": state.plan_epoch,
        "paused_reason": state.paused_reason,
        "done": done,
        "previous_error": previous_error,
        "fact_store": fact_store.map(|store| store.to_projected_planning_value(MAX_FACT_ENTRIES_IN_SUMMARY)),
        "input_slots": input_slots.value,
        "input_registry": build_input_registry_projection(&input_slots.resolved, input_slots.missing.as_slice()),
        "canonical_context": build_canonical_context_projection(&input_slots.resolved),
        "tool_memory_projection": tool_memory_projection,
        "intent_slots": state.runtime.pointer("/agent/intent_grounding"),
        "capability_view": state.runtime.pointer("/agent/capability_view"),
        "capability_ready": state.runtime.pointer("/agent/capability_ready"),
        "side_effect_lifecycle": state.runtime.pointer("/agent/side_effect_lifecycle"),
        "todo_state": state.runtime.pointer("/agent/todo_progress"),
    });
    let adaptive_budget = derive_adaptive_planner_budget(state, token_budget);
    let strategy = resolve_context_compaction_strategy(adaptive_budget);
    let mut budgeted = apply_planner_context_budget(base, adaptive_budget);
    apply_context_pressure_strategy(&mut budgeted, strategy);
    compact_json_with_options(&budgeted, &strategy.final_compact_options)
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextPressureMode {
    Normal,
    Light,
    Medium,
    Critical,
}

impl ContextPressureMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ContextCompactionStrategy {
    mode: ContextPressureMode,
    final_compact_options: JsonBudgetOptions,
    drop_input_slot_canonical_refs: bool,
    drop_capability_protocols: bool,
    drop_last_failed_assistant_content: bool,
    tool_memory_compact_options: Option<JsonBudgetOptions>,
    failed_finalize_compact_options: Option<JsonBudgetOptions>,
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
    let usage_ratio_bps = match (window_input_tokens, soft_limit_tokens) {
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
    let usage_ratio_bps = usage_ratio_bps.or_else(|| match (remaining_tokens, soft_limit_tokens) {
        (Some(remaining), Some(soft_limit)) if soft_limit > 0 => Some(
            soft_limit
                .saturating_sub(remaining.min(soft_limit))
                .saturating_mul(10_000)
                / soft_limit,
        ),
        _ => None,
    });

    let mut effective = base_token_limit.max(1);
    let mut mode = "default";
    if let Some(usage_bps) = usage_ratio_bps {
        if usage_bps < CONTEXT_USAGE_LIGHT_THRESHOLD_BPS {
            let relaxed_cap = base_token_limit
                .saturating_mul(ADAPTIVE_RELAXED_MAX_MULTIPLIER)
                .max(base_token_limit);
            let relaxed_target = base_token_limit.saturating_mul(3).saturating_div(2);
            effective = relaxed_target.max(base_token_limit).min(relaxed_cap).max(1);
            mode = "relaxed";
        } else if usage_bps < CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS {
            effective = base_token_limit.max(1);
            mode = "balanced";
        } else if usage_bps < CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS {
            effective = base_token_limit.saturating_mul(ADAPTIVE_MEDIUM_NUMERATOR)
                / ADAPTIVE_MEDIUM_DENOMINATOR;
            effective = effective.max(1);
            mode = "medium";
        } else {
            effective = base_token_limit.saturating_mul(ADAPTIVE_CRITICAL_NUMERATOR)
                / ADAPTIVE_CRITICAL_DENOMINATOR;
            effective = effective.max(1);
            mode = "tight";
        }
    }

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

fn resolve_context_compaction_strategy(budget: AdaptivePlannerBudget) -> ContextCompactionStrategy {
    let critical = budget
        .usage_ratio_bps
        .is_some_and(|ratio| ratio >= CONTEXT_USAGE_CRITICAL_THRESHOLD_BPS)
        || (budget.usage_ratio_bps.is_none()
            && budget
                .remaining_tokens
                .is_some_and(|remaining| remaining <= CONTEXT_PRESSURE_CRITICAL_REMAINING_TOKENS));
    if critical {
        return ContextCompactionStrategy {
            mode: ContextPressureMode::Critical,
            final_compact_options: JsonBudgetOptions {
                max_depth: 8,
                max_object_entries: 96,
                max_array_items: 64,
                max_string_chars: 1200,
            },
            drop_input_slot_canonical_refs: true,
            drop_capability_protocols: true,
            drop_last_failed_assistant_content: true,
            tool_memory_compact_options: Some(JsonBudgetOptions {
                max_depth: 5,
                max_object_entries: 24,
                max_array_items: 12,
                max_string_chars: 480,
            }),
            failed_finalize_compact_options: Some(JsonBudgetOptions {
                max_depth: 5,
                max_object_entries: 32,
                max_array_items: 12,
                max_string_chars: 560,
            }),
        };
    }

    let medium = budget
        .usage_ratio_bps
        .is_some_and(|ratio| ratio >= CONTEXT_USAGE_MEDIUM_THRESHOLD_BPS)
        || (budget.usage_ratio_bps.is_none()
            && budget
                .remaining_tokens
                .is_some_and(|remaining| remaining <= CONTEXT_PRESSURE_TIGHT_REMAINING_TOKENS));
    if medium {
        return ContextCompactionStrategy {
            mode: ContextPressureMode::Medium,
            final_compact_options: JsonBudgetOptions {
                max_depth: 9,
                max_object_entries: 112,
                max_array_items: 96,
                max_string_chars: 2048,
            },
            drop_input_slot_canonical_refs: true,
            drop_capability_protocols: false,
            drop_last_failed_assistant_content: false,
            tool_memory_compact_options: Some(JsonBudgetOptions {
                max_depth: 6,
                max_object_entries: 36,
                max_array_items: 16,
                max_string_chars: 800,
            }),
            failed_finalize_compact_options: Some(JsonBudgetOptions {
                max_depth: 6,
                max_object_entries: 48,
                max_array_items: 20,
                max_string_chars: 900,
            }),
        };
    }

    let light = budget
        .usage_ratio_bps
        .is_some_and(|ratio| ratio >= CONTEXT_USAGE_LIGHT_THRESHOLD_BPS);
    if light {
        return ContextCompactionStrategy {
            mode: ContextPressureMode::Light,
            final_compact_options: JsonBudgetOptions {
                max_depth: 10,
                max_object_entries: 120,
                max_array_items: 120,
                max_string_chars: 3072,
            },
            drop_input_slot_canonical_refs: false,
            drop_capability_protocols: false,
            drop_last_failed_assistant_content: false,
            tool_memory_compact_options: Some(JsonBudgetOptions {
                max_depth: 8,
                max_object_entries: 96,
                max_array_items: 48,
                max_string_chars: 1800,
            }),
            failed_finalize_compact_options: Some(JsonBudgetOptions {
                max_depth: 8,
                max_object_entries: 96,
                max_array_items: 48,
                max_string_chars: 2000,
            }),
        };
    }

    ContextCompactionStrategy {
        mode: ContextPressureMode::Normal,
        final_compact_options: JsonBudgetOptions {
            max_depth: 10,
            max_object_entries: 128,
            max_array_items: 128,
            max_string_chars: 4096,
        },
        drop_input_slot_canonical_refs: false,
        drop_capability_protocols: false,
        drop_last_failed_assistant_content: false,
        tool_memory_compact_options: None,
        failed_finalize_compact_options: None,
    }
}

#[derive(Debug, Clone)]
struct InputSlotProjection {
    value: Value,
    resolved: BTreeMap<String, Value>,
    missing: Vec<String>,
}

fn build_input_slots_projection(
    state: &EngineRunnerState,
    fact_store: Option<&FactStore>,
) -> InputSlotProjection {
    let mut resolved = BTreeMap::<String, Value>::new();
    if let Some(inputs) = state.runtime.pointer("/inputs") {
        collect_runtime_input_slots(inputs, &mut Vec::new(), &mut resolved);
    }
    if let Some(store) = fact_store {
        let projected = store.to_projected_planning_value(256);
        if let Some(facts) = projected.get("facts").and_then(Value::as_object) {
            let mut keys = facts.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if !key.starts_with("inputs.") {
                    continue;
                }
                let Some(slot) = key.strip_prefix("inputs.") else {
                    continue;
                };
                if slot.trim().is_empty() {
                    continue;
                }
                if let Some(value) = facts.get(key.as_str()) {
                    resolved
                        .entry(slot.to_string())
                        .or_insert_with(|| value.clone());
                }
            }
        }
        if !resolved.contains_key("owner") {
            if let Some(value) = store.get("owner").map(|entry| entry.value.clone()) {
                resolved.insert("owner".to_string(), value);
            }
        }
        if !resolved.contains_key("wallet.default") {
            if let Some(value) = store.get("wallet.default").map(|entry| entry.value.clone()) {
                resolved.insert("wallet.default".to_string(), value);
            }
        }
    }

    let mut missing = BTreeSet::<String>::new();
    if let Some(required_facts) = state
        .runtime
        .pointer("/agent/todo_progress/current_todo/required_facts")
        .and_then(Value::as_array)
    {
        for required in required_facts.iter().filter_map(Value::as_str) {
            if let Some(slot) = to_slot_id(required) {
                if !resolved.contains_key(slot.as_str()) {
                    missing.insert(slot);
                }
            }
        }
    }
    if let Some(questions) = state
        .runtime
        .pointer("/agent/missing_required_input/questions")
        .and_then(Value::as_array)
    {
        for question in questions {
            let Some(slot) = question
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            else {
                continue;
            };
            if !resolved.contains_key(slot.as_str()) {
                missing.insert(slot);
            }
        }
    }

    let mut resolved_items = Vec::<Value>::new();
    let mut canonical_refs = serde_json::Map::<String, Value>::new();
    for (slot, value) in &resolved {
        let canonical_ref = format!("inputs.{slot}");
        resolved_items.push(json!({
            "id": slot,
            "ref": canonical_ref,
            "value": value,
        }));
        canonical_refs.insert(slot.clone(), Value::String(canonical_ref));
    }
    let missing_items = missing
        .into_iter()
        .map(|slot| {
            json!({
                "id": slot,
                "ref": format!("inputs.{slot}"),
            })
        })
        .collect::<Vec<_>>();
    let missing_slots = missing_items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    let missing_count = missing_items.len();

    let value = json!({
        "resolved": resolved_items,
        "missing": missing_items,
        "canonical_refs": canonical_refs,
        "counts": {
            "resolved": resolved.len(),
            "missing": missing_count,
        }
    });
    InputSlotProjection {
        value,
        resolved,
        missing: missing_slots,
    }
}

fn build_canonical_context_projection(resolved: &BTreeMap<String, Value>) -> Value {
    let mut chain_refs = Vec::<Value>::new();
    let mut account_refs = Vec::<Value>::new();
    let mut asset_refs = Vec::<Value>::new();
    let mut amount_refs = Vec::<Value>::new();

    let mut chain_seen = BTreeSet::<String>::new();
    let mut account_seen = BTreeSet::<String>::new();
    let mut asset_seen = BTreeSet::<String>::new();
    let mut amount_seen = BTreeSet::<String>::new();

    for (slot, value) in resolved {
        let canonical_ref = format!("inputs.{slot}");
        if let Some(chain_ref) = extract_chain_ref(slot, value) {
            let dedupe = format!("{slot}:{chain_ref}");
            if chain_seen.insert(dedupe) {
                chain_refs.push(json!({
                    "id": slot,
                    "ref": canonical_ref,
                    "chain_ref": chain_ref,
                }));
            }
        }

        if let Some(account_ref) = extract_account_ref(slot, value) {
            let dedupe = format!("{slot}:{account_ref}");
            if account_seen.insert(dedupe) {
                account_refs.push(json!({
                    "id": slot,
                    "ref": canonical_ref,
                    "role": slot_leaf(slot),
                    "account_ref": account_ref,
                }));
            }
        }

        if let Some(asset_ref) = extract_asset_ref(value) {
            let dedupe = format!(
                "{slot}:{}:{}",
                asset_ref
                    .get("address")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                asset_ref
                    .get("chain_ref")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            if asset_seen.insert(dedupe) {
                let mut item = serde_json::Map::<String, Value>::new();
                item.insert("id".to_string(), Value::String(slot.to_string()));
                item.insert("ref".to_string(), Value::String(canonical_ref.clone()));
                for (key, value) in asset_ref {
                    item.insert(key, value);
                }
                asset_refs.push(Value::Object(item));
            }
        }

        if let Some(amount_ref) = extract_amount_ref(slot, value) {
            let encoded = serde_json::to_string(&amount_ref).unwrap_or_else(|_| "{}".to_string());
            let dedupe = format!("{slot}:{encoded}");
            if amount_seen.insert(dedupe) {
                let mut item = serde_json::Map::<String, Value>::new();
                item.insert("id".to_string(), Value::String(slot.to_string()));
                item.insert("ref".to_string(), Value::String(canonical_ref));
                for (key, value) in amount_ref {
                    item.insert(key, value);
                }
                amount_refs.push(Value::Object(item));
            }
        }
    }

    json!({
        "chain_refs": chain_refs,
        "account_refs": account_refs,
        "asset_refs": asset_refs,
        "amount_refs": amount_refs,
        "counts": {
            "chain_refs": chain_seen.len(),
            "account_refs": account_seen.len(),
            "asset_refs": asset_seen.len(),
            "amount_refs": amount_seen.len(),
        }
    })
}

fn build_input_registry_projection(
    resolved: &BTreeMap<String, Value>,
    missing: &[String],
) -> Value {
    let mut known_refs = BTreeSet::<String>::new();
    let mut entries = Vec::<Value>::new();
    for (slot, value) in resolved {
        let reference = format!("inputs.{slot}");
        known_refs.insert(reference.clone());
        entries.push(json!({
            "id": slot,
            "ref": reference,
            "status": "resolved",
            "type_hint": value_type_hint(value),
            "example": value,
        }));
    }
    for slot in missing {
        let reference = format!("inputs.{slot}");
        known_refs.insert(reference.clone());
        entries.push(json!({
            "id": slot,
            "ref": reference,
            "status": "missing",
            "required": true,
        }));
    }
    entries.sort_by(|left, right| {
        let left_ref = left.get("ref").and_then(Value::as_str).unwrap_or_default();
        let right_ref = right.get("ref").and_then(Value::as_str).unwrap_or_default();
        left_ref.cmp(right_ref)
    });
    json!({
        "schema": "ais-agent-input-registry/0.0.1",
        "entries": entries,
        "known_refs": known_refs.iter().cloned().collect::<Vec<_>>(),
        "counts": {
            "known_refs": known_refs.len(),
            "resolved": resolved.len(),
            "missing": missing.len(),
        }
    })
}

fn value_type_hint(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn collect_runtime_input_slots(
    value: &Value,
    path: &mut Vec<String>,
    out: &mut BTreeMap<String, Value>,
) {
    if !path.is_empty() {
        out.insert(path.join("."), value.clone());
    }
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get(key.as_str()) else {
                    continue;
                };
                path.push(key);
                collect_runtime_input_slots(child, path, out);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_runtime_input_slots(child, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn extract_chain_ref(slot: &str, value: &Value) -> Option<String> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    if !matches!(
        leaf.as_str(),
        "chain" | "chain_id" | "chain_ref" | "network"
    ) {
        return None;
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_account_ref(slot: &str, value: &Value) -> Option<String> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    let hint = [
        "owner",
        "from",
        "sender",
        "recipient",
        "to",
        "wallet",
        "account",
        "payer",
        "authority",
    ]
    .iter()
    .any(|name| leaf == *name || leaf.ends_with(name));
    if !hint {
        return None;
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn extract_asset_ref(value: &Value) -> Option<serde_json::Map<String, Value>> {
    let object = value.as_object()?;
    let address = object
        .get("address")
        .or_else(|| object.get("mint"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut out = serde_json::Map::<String, Value>::new();
    out.insert("address".to_string(), Value::String(address.to_string()));
    if let Some(chain_ref) = object
        .get("chain_ref")
        .or_else(|| object.get("chain_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.insert(
            "chain_ref".to_string(),
            Value::String(chain_ref.to_string()),
        );
    }
    if let Some(symbol) = object
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.insert("symbol".to_string(), Value::String(symbol.to_string()));
    }
    if let Some(decimals) = object.get("decimals").and_then(Value::as_u64) {
        out.insert("decimals".to_string(), Value::Number(decimals.into()));
    }
    Some(out)
}

fn extract_amount_ref(slot: &str, value: &Value) -> Option<serde_json::Map<String, Value>> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    let is_amount_slot =
        leaf.contains("amount") || matches!(leaf.as_str(), "value" | "qty" | "quantity" | "size");
    if !is_amount_slot {
        return None;
    }
    let mut out = serde_json::Map::<String, Value>::new();
    match value {
        Value::Null | Value::Bool(_) => None,
        Value::Object(object) => {
            if let Some(human) = object.get("human") {
                out.insert("amount_human".to_string(), human.clone());
            }
            if let Some(atomic) = object.get("atomic") {
                out.insert("amount_atomic".to_string(), atomic.clone());
            }
            if out.is_empty() {
                out.insert("amount_ref".to_string(), value.clone());
            }
            Some(out)
        }
        _ => {
            out.insert("amount_ref".to_string(), value.clone());
            Some(out)
        }
    }
}

fn slot_leaf(slot: &str) -> &str {
    slot.rsplit('.').next().unwrap_or(slot)
}

fn to_slot_id(required_fact: &str) -> Option<String> {
    let trimmed = required_fact.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(slot) = trimmed.strip_prefix("inputs.") {
        let slot = slot.trim();
        if slot.is_empty() {
            return None;
        }
        return Some(slot.to_string());
    }
    if trimmed.starts_with("facts.")
        || trimmed.starts_with("tx.")
        || trimmed.starts_with("nodes.")
        || trimmed.starts_with("query.")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn apply_planner_context_budget(mut base: Value, budget: AdaptivePlannerBudget) -> Value {
    let token_budget = budget.effective_token_limit.max(1);
    let baseline_tokens = estimate_tokens_json(&base);
    let mut selected_stage = PLANNER_CONTEXT_STAGES[PLANNER_CONTEXT_STAGES.len() - 1];
    let mut selected_tokens = baseline_tokens;
    let mut selected = base.clone();
    let mut truncated = false;

    for stage in PLANNER_CONTEXT_STAGES {
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
                "estimated_tokens": selected_tokens,
                "baseline_estimated_tokens": baseline_tokens,
                "estimator": "chars_div_4",
                "stage": selected_stage.name,
                "truncated": truncated,
            }),
        );
    }
    base = selected;
    base
}

fn apply_context_pressure_strategy(summary: &mut Value, strategy: ContextCompactionStrategy) {
    let mut actions = Vec::<String>::new();
    if strategy.drop_input_slot_canonical_refs
        && set_pointer_value(summary, "/input_slots/canonical_refs", Value::Null)
    {
        actions.push("drop_input_slots_canonical_refs".to_string());
    }

    if strategy.drop_capability_protocols {
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

    if let Some(options) = strategy.tool_memory_compact_options {
        if compact_pointer_value(summary, "/tool_memory_projection", options) {
            actions.push("compress_tool_memory_projection".to_string());
        }
    }

    if let Some(options) = strategy.failed_finalize_compact_options {
        if compact_pointer_value(summary, "/previous_error/last_failed_finalize", options) {
            actions.push("compress_last_failed_finalize".to_string());
        }
    }

    if strategy.drop_last_failed_assistant_content
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

fn apply_context_stage(summary: &mut Value, stage: PlannerContextStage) {
    trim_fact_store(summary, stage.max_fact_entries);
    trim_input_slots(
        summary,
        stage.max_input_slots_resolved,
        stage.max_input_slots_missing,
    );
    trim_input_registry(summary, stage.max_registry_entries, stage.max_known_refs);
    trim_canonical_context(summary, stage.max_canonical_per_group);
    trim_capability_view(summary, stage);
    trim_previous_error(summary, stage.max_previous_error_issues);
}

fn trim_fact_store(summary: &mut Value, max_entries: usize) {
    let facts = summary
        .pointer("/fact_store/facts")
        .and_then(Value::as_object)
        .cloned();
    let meta = summary
        .pointer("/fact_store/meta")
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

    if let Some(slot) = summary.pointer_mut("/fact_store/facts") {
        *slot = Value::Object(projected_facts);
    }
    if let Some(slot) = summary.pointer_mut("/fact_store/meta") {
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

fn trim_capability_view(summary: &mut Value, stage: PlannerContextStage) {
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

fn prioritize_keys(mut keys: Vec<String>, max_entries: usize) -> Vec<String> {
    keys.sort_by(|left, right| {
        slot_sort_key(left.as_str())
            .cmp(&slot_sort_key(right.as_str()))
            .then_with(|| left.cmp(right))
    });
    keys.dedup();
    keys.truncate(max_entries.max(1));
    keys
}

fn registry_order_key(entry: &Value) -> (u8, u8, String) {
    let status_rank = match entry.get("status").and_then(Value::as_str) {
        Some("missing") => 0,
        Some("resolved") => 1,
        _ => 2,
    };
    let slot = entry
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| entry.get("ref").and_then(Value::as_str))
        .unwrap_or_default();
    (
        status_rank,
        slot_sort_key(slot),
        entry
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

fn slot_order_key(value: &Value) -> (u8, String) {
    if let Some(slot) = value
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| value.get("ref").and_then(Value::as_str))
    {
        return (slot_sort_key(slot), slot.to_string());
    }
    if let Some(text) = value.as_str() {
        return (slot_sort_key(text), text.to_string());
    }
    let encoded = serde_json::to_string(value).unwrap_or_default();
    (slot_sort_key(encoded.as_str()), encoded)
}

fn slot_sort_key(slot: &str) -> u8 {
    let lowered = slot.to_ascii_lowercase();
    if PRIORITY_SLOTS
        .iter()
        .any(|priority| lowered == *priority || lowered.ends_with(priority))
    {
        return 0;
    }
    if lowered.contains("owner")
        || lowered.contains("wallet")
        || lowered.contains("recipient")
        || lowered.contains("token")
        || lowered.contains("asset")
        || lowered.contains("amount")
        || lowered.contains("chain")
    {
        return 1;
    }
    2
}

fn estimate_tokens_json(value: &Value) -> u64 {
    let encoded = serde_json::to_string(value).unwrap_or_default();
    let chars = encoded.chars().count();
    chars
        .saturating_add(CHARS_PER_TOKEN_ESTIMATE.saturating_sub(1))
        .checked_div(CHARS_PER_TOKEN_ESTIMATE)
        .unwrap_or(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::facts::{FactLayer, FactSource};
    use serde_json::json;

    #[test]
    fn planning_context_marks_unchanged_payloads() {
        let mut manager = PlanningContextManager::default();
        let state = EngineRunnerState::default();
        let first = manager.next_summary(&state, 0, false, None, None, None);
        assert_eq!(
            first.pointer("/context_unchanged"),
            Some(&Value::Bool(false))
        );
        let second = manager.next_summary(&state, 0, false, None, None, None);
        assert_eq!(
            second.pointer("/context_unchanged"),
            Some(&Value::Bool(true))
        );
        assert!(
            second.pointer("/input_registry").is_some(),
            "unchanged summaries must still include full projected context"
        );
    }

    #[test]
    fn projected_summary_includes_tool_memory_projection() {
        let state = EngineRunnerState::default();
        let tool_memory_projection = json!({
            "schema": "ais-agent-tool-memory-projection/0.0.1",
            "recent": {
                "catalog_search": [
                    {"query":"transfer","top_refs":[{"ref":"erc20@0.0.2/transfer"}]}
                ],
                "candidate_detail": [],
                "guide": {"schema": {}, "topic": {"cel": {}}}
            }
        });
        let summary =
            build_projected_summary(&state, 0, false, None, None, Some(&tool_memory_projection));
        assert_eq!(
            summary.pointer("/tool_memory_projection/schema"),
            Some(&json!("ais-agent-tool-memory-projection/0.0.1"))
        );
        assert_eq!(
            summary.pointer("/tool_memory_projection/recent/guide/topic/cel"),
            Some(&json!({}))
        );
    }

    #[test]
    fn projected_summary_limits_fact_entries() {
        let mut store = FactStore::default();
        for index in 0..40 {
            store.upsert(
                format!("k.{index}"),
                json!(index),
                FactLayer::Seed,
                FactSource::RuntimeProvided,
                "runtime.inputs",
            );
        }
        store.upsert(
            "owner",
            json!("0xabc"),
            FactLayer::Seed,
            FactSource::RuntimeProvided,
            "runtime.inputs.owner",
        );
        let state = EngineRunnerState::default();
        let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
        assert_eq!(
            summary.pointer("/fact_store/facts/owner"),
            Some(&json!("0xabc"))
        );
        assert!(
            summary
                .pointer("/fact_store/meta/_truncated_entries")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        );
    }

    #[test]
    fn projected_summary_includes_input_slots_and_missing_refs() {
        let mut store = FactStore::default();
        store.upsert(
            "inputs.owner",
            json!("0xabc"),
            FactLayer::Seed,
            FactSource::RuntimeProvided,
            "runtime.inputs.owner",
        );
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": {
                    "owner": "0xabc",
                    "token": {"address":"0xdef"}
                },
                "agent": {
                    "todo_progress": {
                        "current_todo": {
                            "required_facts": ["inputs.owner", "inputs.amount"]
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
        assert_eq!(
            summary.pointer("/input_slots/canonical_refs/owner"),
            Some(&json!("inputs.owner"))
        );
        assert_eq!(
            summary.pointer("/input_slots/canonical_refs/token.address"),
            Some(&json!("inputs.token.address"))
        );
        assert_eq!(
            summary.pointer("/input_slots/missing/0/ref"),
            Some(&json!("inputs.amount"))
        );
        assert_eq!(
            summary.pointer("/input_registry/known_refs/0"),
            Some(&json!("inputs.amount"))
        );
        assert_eq!(
            summary.pointer("/input_registry/known_refs/1"),
            Some(&json!("inputs.owner"))
        );
        assert_eq!(
            summary.pointer("/input_registry/entries/0/status"),
            Some(&json!("missing"))
        );
        assert_eq!(
            summary.pointer("/canonical_context/account_refs/0/account_ref"),
            Some(&json!("0xabc"))
        );
    }

    #[test]
    fn projected_summary_includes_chain_account_asset_and_amount_refs() {
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": {
                    "chain_id": "eip155:31338",
                    "owner": "0x1111111111111111111111111111111111111111",
                    "recipient": "0x2222222222222222222222222222222222222222",
                    "token": {
                        "address":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
                        "chain_id":"eip155:31338",
                        "decimals": 18,
                        "symbol": "TKN"
                    },
                    "amount": {
                        "human":"1.25",
                        "atomic":"1250000000000000000"
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let summary = build_projected_summary(&state, 0, false, None, None, None);
        assert_eq!(
            summary.pointer("/canonical_context/chain_refs/0/chain_ref"),
            Some(&json!("eip155:31338"))
        );
        assert_eq!(
            summary.pointer("/canonical_context/account_refs/0/account_ref"),
            Some(&json!("0x1111111111111111111111111111111111111111"))
        );
        assert_eq!(
            summary.pointer("/canonical_context/account_refs/1/account_ref"),
            Some(&json!("0x2222222222222222222222222222222222222222"))
        );
        assert_eq!(
            summary.pointer("/canonical_context/asset_refs/0/chain_ref"),
            Some(&json!("eip155:31338"))
        );
        assert_eq!(
            summary.pointer("/canonical_context/amount_refs/0/amount_atomic"),
            Some(&json!("1250000000000000000"))
        );
    }

    #[test]
    fn projected_summary_applies_budget_and_keeps_priority_slots() {
        let mut inputs = serde_json::Map::<String, Value>::new();
        inputs.insert("owner".to_string(), json!("0xabc"));
        for index in 0..220 {
            inputs.insert(format!("extra_{index}"), json!(format!("v{index}")));
        }
        let mut protocols = Vec::<Value>::new();
        for index in 0..120 {
            protocols.push(json!({
                "protocol": format!("protocol-{index:03}"),
                "chains": ["eip155:1"],
                "actions": [{"name":"a","ref":format!("p{index}/a")}],
                "queries": [{"name":"q","ref":format!("p{index}/q")}],
                "required_inputs": ["owner", "amount"]
            }));
        }
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": Value::Object(inputs),
                "agent": {
                    "capability_view": {
                        "schema": "ais-agent-capability-view/0.0.1",
                        "ready": true,
                        "protocols": protocols,
                        "counts": {
                            "protocols": 120,
                            "actions": 120,
                            "queries": 120
                        }
                    },
                    "todo_progress": {
                        "current_todo": {
                            "required_facts": ["inputs.owner", "inputs.amount"]
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let mut store = FactStore::default();
        for index in 0..120 {
            store.upsert(
                format!("inputs.extra_{index}"),
                json!(index),
                FactLayer::Seed,
                FactSource::RuntimeProvided,
                "runtime.inputs",
            );
        }
        store.upsert(
            "owner",
            json!("0xabc"),
            FactLayer::Seed,
            FactSource::RuntimeProvided,
            "runtime.inputs.owner",
        );

        let summary = build_projected_summary(&state, 0, false, None, Some(&store), None);
        assert_eq!(
            summary.pointer("/input_registry/entries/0/ref"),
            Some(&json!("inputs.amount"))
        );
        assert_eq!(
            summary.pointer("/fact_store/facts/owner"),
            Some(&json!("0xabc"))
        );
        assert_eq!(
            summary.pointer("/context_budget/token_limit"),
            Some(&json!(DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET))
        );
        assert_eq!(
            summary.pointer("/context_budget/adaptive_mode"),
            Some(&json!("default"))
        );
        assert_eq!(
            summary.pointer("/context_budget/estimator"),
            Some(&json!("chars_div_4"))
        );
        assert!(
            summary
                .pointer("/context_budget/truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "large context should be marked truncated"
        );
    }

    #[test]
    fn projected_summary_relaxes_budget_when_context_remaining_is_high() {
        let mut inputs = serde_json::Map::<String, Value>::new();
        inputs.insert("owner".to_string(), json!("0xabc"));
        for index in 0..120 {
            inputs.insert(format!("extra_{index}"), json!(format!("v{index}")));
        }
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": Value::Object(inputs),
                "agent": {
                    "llm_usage": {
                        "context_soft_limit_tokens": 100000,
                        "context_remaining_tokens": 90000
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let summary = build_projected_summary(&state, 0, false, None, None, None);
        let token_limit = summary
            .pointer("/context_budget/token_limit")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert!(token_limit > DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET as u64);
        assert_eq!(
            summary.pointer("/context_budget/adaptive_mode"),
            Some(&json!("relaxed"))
        );
        assert_eq!(
            summary.pointer("/context_budget/adaptive/remaining_ratio_bps"),
            Some(&json!(9000))
        );
    }

    #[test]
    fn projected_summary_uses_critical_pressure_strategy_when_usage_exceeds_ninety_percent() {
        let long_text = "x".repeat(4000);
        let state = EngineRunnerState {
            runtime: json!({
                "inputs": {
                    "owner": "0xabc",
                    "token": {"address":"0xdef"}
                },
                "agent": {
                    "llm_usage": {
                        "context_soft_limit_tokens": 100000,
                        "context_remaining_tokens": 900
                    },
                    "capability_view": {
                        "schema": "ais-agent-capability-view/0.0.1",
                        "ready": true,
                        "protocols": [{
                            "protocol": "erc20@0.0.2",
                            "actions": [{"name":"transfer","ref":"erc20@0.0.2/transfer"}],
                            "queries": [{"name":"balance-of","ref":"erc20@0.0.2/balance-of"}],
                            "required_inputs": ["owner", "recipient"]
                        }],
                        "topics": ["transfer"]
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let previous_error = json!({
            "phase": "planning",
            "reason_code": "planner_invalid_tool_output",
            "last_failed_finalize": {
                "tool": "plan.revise_segment",
                "arguments": {
                    "status":"proposed",
                    "segment": {
                        "segment_id":"seg_1",
                        "steps":[{"id":"s1","kind":"query","inputs":{"owner":{"ref":"inputs.owner"}}}]
                    }
                },
                "assistant_content": long_text
            }
        });
        let tool_memory_projection = json!({
            "schema": "ais-agent-tool-memory-projection/0.0.1",
            "recent": {
                "guide": {
                    "schema": {
                        "ais-plan-sketch/0.1.0": {
                            "summary": "y".repeat(5000)
                        }
                    },
                    "topic": {}
                }
            }
        });
        let summary = build_projected_summary(
            &state,
            0,
            false,
            Some(&previous_error),
            None,
            Some(&tool_memory_projection),
        );
        assert_eq!(
            summary.pointer("/context_budget/pressure_mode"),
            Some(&json!("critical"))
        );
        assert_eq!(
            summary.pointer("/input_slots/canonical_refs"),
            Some(&Value::Null)
        );
        assert_eq!(
            summary.pointer("/capability_view/protocols"),
            Some(&json!([]))
        );
        assert_eq!(
            summary.pointer("/previous_error/last_failed_finalize/assistant_content"),
            Some(&Value::Null)
        );
        let actions = summary
            .pointer("/context_budget/pressure_actions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(actions
            .iter()
            .any(|item| item.as_str() == Some("drop_capability_protocols")));
        assert!(actions
            .iter()
            .any(|item| item.as_str() == Some("compress_tool_memory_projection")));
    }
}
