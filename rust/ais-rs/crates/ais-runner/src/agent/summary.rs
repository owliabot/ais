use ais_engine::{EngineEventRecord, EngineEventType, EngineRunnerState};
use ais_sdk::PlanDocument;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PauseKind {
    NeedUserConfirm,
    NeedUserInput,
    HardBlock,
    ConditionFailed,
    ExecutorError,
    AssertFailed,
    Cancelled,
    NoProgress,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedUserConfirmSummary {
    pub reason_code: Option<String>,
    pub reason: Option<String>,
    pub confirmation_hash: Option<String>,
    pub confirmation_summary: Option<Value>,
    pub segment_bundle: Vec<NeedUserConfirmBundleItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedUserConfirmBundleItem {
    pub node_id: String,
    pub action_ref: Option<String>,
    pub chain: Option<String>,
    pub execution_type: Option<String>,
    pub risk_level: Option<u64>,
    pub params: Vec<NeedUserConfirmParamItem>,
    pub confirmation_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedUserConfirmParamItem {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PauseSummary {
    pub raw_reason: Option<String>,
    pub kind: PauseKind,
    pub node_id: Option<String>,
    pub need_user_confirm: Option<NeedUserConfirmSummary>,
    pub last_error_reason: Option<String>,
}

#[allow(dead_code)]
pub fn summarize_pause(paused_reason: Option<&str>, events: &[EngineEventRecord]) -> PauseSummary {
    summarize_pause_with_context(paused_reason, events, None, None)
}

pub fn summarize_pause_with_context(
    paused_reason: Option<&str>,
    events: &[EngineEventRecord],
    plan: Option<&PlanDocument>,
    state: Option<&EngineRunnerState>,
) -> PauseSummary {
    let (kind, node_id) = parse_paused_reason(paused_reason);
    let last_error_reason = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::Error)
        .and_then(|record| {
            record
                .event
                .data
                .get("reason_code")
                .or_else(|| record.event.data.get("reason"))
        })
        .and_then(Value::as_str)
        .map(str::to_string);
    let need_user_confirm = events
        .iter()
        .rev()
        .find(|record| record.event.event_type == EngineEventType::NeedUserConfirm)
        .map(|record| {
            let reason = record
                .event
                .data
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string);
            let reason_code = record
                .event
                .data
                .get("reason_code")
                .and_then(Value::as_str)
                .map(str::to_string);
            let confirmation_hash = record
                .event
                .data
                .get("details")
                .and_then(Value::as_object)
                .and_then(|details| details.get("confirmation_hash"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let confirmation_summary = record
                .event
                .data
                .get("details")
                .and_then(Value::as_object)
                .and_then(|details| details.get("confirmation_summary"))
                .cloned();
            let segment_bundle = node_id
                .as_deref()
                .and_then(segment_id_from_node_id)
                .map(|segment_id| {
                    build_segment_confirm_bundle(
                        segment_id,
                        plan,
                        state,
                        confirmation_summary.as_ref(),
                        confirmation_hash.as_deref(),
                    )
                })
                .unwrap_or_default();
            NeedUserConfirmSummary {
                reason_code,
                reason,
                confirmation_hash,
                confirmation_summary,
                segment_bundle,
            }
        });

    PauseSummary {
        raw_reason: paused_reason.map(str::to_string),
        kind,
        node_id,
        need_user_confirm,
        last_error_reason,
    }
}

fn parse_paused_reason(paused_reason: Option<&str>) -> (PauseKind, Option<String>) {
    let Some(reason) = paused_reason else {
        return (PauseKind::Other, None);
    };
    let (prefix, rest) = reason
        .split_once(':')
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((reason, None));
    let node_id = rest.map(str::to_string);
    let kind = match prefix {
        "need_user_confirm" => PauseKind::NeedUserConfirm,
        "need_user_input" | "missing_required_input" => PauseKind::NeedUserInput,
        "hard_block" => PauseKind::HardBlock,
        "condition_failed" => PauseKind::ConditionFailed,
        "executor_error" => PauseKind::ExecutorError,
        "assert_failed" => PauseKind::AssertFailed,
        "cancelled_by_command" => PauseKind::Cancelled,
        "no_progress" => PauseKind::NoProgress,
        _ => PauseKind::Other,
    };
    (kind, node_id)
}

impl PauseSummary {
    pub fn render_for_humans(&self) -> String {
        let mut out = String::new();
        out.push_str("[agent] engine paused\n");
        out.push_str(
            format!(
                "- paused_reason: {}\n",
                self.raw_reason.as_deref().unwrap_or("none")
            )
            .as_str(),
        );
        out.push_str(format!("- kind: {:?}\n", self.kind).as_str());
        out.push_str(
            format!("- node_id: {}\n", self.node_id.as_deref().unwrap_or("none")).as_str(),
        );
        if let Some(error) = &self.last_error_reason {
            out.push_str(format!("- last_error: {error}\n").as_str());
        }
        if let Some(need) = &self.need_user_confirm {
            if let Some(reason_code) = &need.reason_code {
                out.push_str(format!("- need_user_confirm.reason_code: {reason_code}\n").as_str());
            }
            if let Some(reason) = &need.reason {
                out.push_str(format!("- need_user_confirm.reason: {reason}\n").as_str());
            }
            if let Some(hash) = &need.confirmation_hash {
                out.push_str(format!("- confirmation_hash: {hash}\n").as_str());
            }
            if let Some(summary) = need.confirmation_summary.as_ref() {
                if let Some(chain) = summary.get("chain").and_then(Value::as_str) {
                    out.push_str(format!("- chain: {chain}\n").as_str());
                }
                if let Some(action_ref) = summary.get("action_ref").and_then(Value::as_str) {
                    out.push_str(format!("- action_ref: {action_ref}\n").as_str());
                }
                if let Some(execution_type) = summary.get("execution_type").and_then(Value::as_str)
                {
                    out.push_str(format!("- execution_type: {execution_type}\n").as_str());
                }
                if let Some(risk_level) = summary.get("risk_level").and_then(Value::as_u64) {
                    out.push_str(format!("- risk_level: {risk_level}\n").as_str());
                }
                if let Some(details) = summary.get("details").and_then(Value::as_object) {
                    if let Some(amount) = first_string(
                        details,
                        &["spend_amount", "amount", "value", "approval_amount"],
                    ) {
                        out.push_str(format!("- amount: {amount}\n").as_str());
                    }
                    if let Some(asset) =
                        first_string(details, &["asset", "token", "token_address", "symbol"])
                    {
                        out.push_str(format!("- asset: {asset}\n").as_str());
                    }
                    if let Some(to) = first_string(
                        details,
                        &[
                            "to",
                            "to_address",
                            "target",
                            "target_address",
                            "recipient",
                            "spender_address",
                        ],
                    ) {
                        out.push_str(format!("- target: {to}\n").as_str());
                    }
                }
            }
            if !need.segment_bundle.is_empty() {
                out.push_str("- segment_confirm_bundle:\n");
                for (index, item) in need.segment_bundle.iter().enumerate() {
                    out.push_str(
                        format!(
                            "  {}. node={} action_ref={} risk={} params={} confirmation_hash={}\n",
                            index + 1,
                            item.node_id,
                            item.action_ref.as_deref().unwrap_or("-"),
                            item.risk_level
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                            render_param_items(item.params.as_slice()),
                            item.confirmation_hash.as_deref().unwrap_or("-"),
                        )
                        .as_str(),
                    );
                }
            }
        }
        out
    }
}

fn build_segment_confirm_bundle(
    segment_id: &str,
    plan: Option<&PlanDocument>,
    state: Option<&EngineRunnerState>,
    current_confirmation_summary: Option<&Value>,
    current_confirmation_hash: Option<&str>,
) -> Vec<NeedUserConfirmBundleItem> {
    let Some(plan_value) = plan.and_then(|plan| serde_json::to_value(plan).ok()) else {
        return Vec::new();
    };
    let completed = state
        .map(|runtime| {
            runtime
                .completed_node_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let approved = state
        .map(|runtime| {
            runtime
                .approved_node_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let nodes = plan_value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut bundle = Vec::<NeedUserConfirmBundleItem>::new();
    for node in nodes {
        let Some(node_id) = node.get("id").and_then(Value::as_str) else {
            continue;
        };
        if segment_id_from_node_id(node_id) != Some(segment_id) {
            continue;
        }
        if node.get("kind").and_then(Value::as_str) != Some("action_ref") {
            continue;
        }
        if completed.contains(node_id) {
            continue;
        }
        if approved.contains(node_id) {
            continue;
        }
        let action_ref = node
            .pointer("/extensions/plan_sketch/candidate_ref")
            .and_then(Value::as_str)
            .map(str::to_string);
        let execution_type = node
            .pointer("/execution/type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let chain = node.get("chain").and_then(Value::as_str).map(str::to_string);
        let risk_level = node
            .pointer("/extensions/risk_level")
            .and_then(Value::as_u64);
        let params = extract_param_bindings(node.get("bindings"), state);
        let confirmation_hash = current_confirmation_summary
            .and_then(|summary| summary.get("node_id").and_then(Value::as_str))
            .filter(|id| *id == node_id)
            .and(current_confirmation_hash.map(str::to_string));
        bundle.push(NeedUserConfirmBundleItem {
            node_id: node_id.to_string(),
            action_ref,
            chain,
            execution_type,
            risk_level,
            params,
            confirmation_hash,
        });
    }
    bundle
}

fn extract_param_bindings(
    bindings: Option<&Value>,
    state: Option<&EngineRunnerState>,
) -> Vec<NeedUserConfirmParamItem> {
    let mut params = Vec::<NeedUserConfirmParamItem>::new();
    let Some(param_map) = bindings
        .and_then(|item| item.get("params"))
        .and_then(Value::as_object)
    else {
        return params;
    };
    let mut keys = param_map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let Some(raw_value) = param_map.get(key.as_str()) else {
            continue;
        };
        if let Some(value) = render_param_value(raw_value, state) {
            params.push(NeedUserConfirmParamItem { key, value });
        }
    }
    params
}

fn render_param_value(value: &Value, state: Option<&EngineRunnerState>) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if value.is_number() || value.is_boolean() {
        return Some(value.to_string());
    }
    let object = value.as_object()?;
    if let Some(lit) = object.get("lit") {
        if let Some(text) = lit.as_str() {
            return Some(text.to_string());
        }
        return Some(lit.to_string());
    }
    if let Some(reference) = object.get("ref").and_then(Value::as_str) {
        return resolve_reference_display(reference, state)
            .or_else(|| Some(format!("ref:{reference}")));
    }
    if let Some(inner) = object.get("object") {
        if let Some(address_ref) = inner
            .get("address")
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("ref"))
            .and_then(Value::as_str)
        {
            return resolve_reference_display(address_ref, state)
                .or_else(|| Some(format!("ref:{address_ref}")));
        }
        if let Some(text) = inner.as_str() {
            return Some(text.to_string());
        }
    }
    if object.get("cel").is_some() {
        return Some("computed(cel)".to_string());
    }
    Some(value.to_string())
}

fn resolve_reference_display(reference: &str, state: Option<&EngineRunnerState>) -> Option<String> {
    let state = state?;
    let key = reference.strip_prefix("inputs.")?;
    state
        .runtime
        .pointer("/agent/state_summary/input_store/facts")
        .and_then(|facts| value_at_dotted_path(facts, key))
        .or_else(|| {
            state
                .runtime
                .pointer("/agent/state_summary/intent_context/facts")
                .and_then(|facts| value_at_dotted_path(facts, key))
        })
        .and_then(value_to_text)
}

fn value_at_dotted_path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in dotted.split('.').filter(|part| !part.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

fn value_to_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if value.is_number() || value.is_boolean() {
        return Some(value.to_string());
    }
    None
}

fn render_param_items(items: &[NeedUserConfirmParamItem]) -> String {
    if items.is_empty() {
        return "-".to_string();
    }
    items
        .iter()
        .map(|item| format!("{}={}", item.key, item.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn segment_id_from_node_id(node_id: &str) -> Option<&str> {
    node_id
        .split_once("__")
        .map(|(segment_id, _)| segment_id)
        .or_else(|| node_id.split_once('/').map(|(segment_id, _)| segment_id))
}

fn first_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            if let Some(text) = value.as_str() {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            } else if value.is_number() || value.is_boolean() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/summary.rs"]
mod tests;
