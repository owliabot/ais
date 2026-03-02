use ais_engine::{EngineEventRecord, EngineEventType};
use serde::Serialize;
use serde_json::Value;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PauseSummary {
    pub raw_reason: Option<String>,
    pub kind: PauseKind,
    pub node_id: Option<String>,
    pub need_user_confirm: Option<NeedUserConfirmSummary>,
    pub last_error_reason: Option<String>,
}

pub fn summarize_pause(paused_reason: Option<&str>, events: &[EngineEventRecord]) -> PauseSummary {
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
            NeedUserConfirmSummary {
                reason_code,
                reason,
                confirmation_hash,
                confirmation_summary,
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
        }
        out
    }
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
