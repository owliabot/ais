use super::{PolicyGateInput, PolicyGateOutput};
use ais_core::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationSummary {
    pub kind: String,
    pub reason: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub chain: String,
    #[serde(default)]
    pub action_ref: Option<String>,
    #[serde(default)]
    pub execution_type: Option<String>,
    #[serde(default)]
    pub risk_level: Option<u8>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub unknown_fields: Vec<String>,
    #[serde(default)]
    pub hard_block_fields: Vec<String>,
    #[serde(default)]
    pub details: Map<String, Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfirmationHashError {
    #[error("confirmation summary hashing failed: {0}")]
    Hash(#[from] serde_json::Error),
}

pub fn build_confirmation_summary(
    gate_input: &PolicyGateInput,
    gate_output: &PolicyGateOutput,
) -> Option<ConfirmationSummary> {
    match gate_output {
        PolicyGateOutput::NeedUserConfirm { reason, details } => Some(ConfirmationSummary {
            kind: "need_user_confirm".to_string(),
            reason: reason.clone(),
            node_id: gate_input.node_id.clone(),
            chain: gate_input.chain.clone(),
            action_ref: gate_input.action_ref.clone(),
            execution_type: gate_input.execution_type.clone(),
            risk_level: gate_input.risk_level,
            risk_tags: gate_input.risk_tags.clone(),
            missing_fields: gate_input.missing_fields.clone(),
            unknown_fields: gate_input.unknown_fields.clone(),
            hard_block_fields: gate_input.hard_block_fields.clone(),
            details: details.clone(),
        }),
        _ => None,
    }
}

pub fn confirmation_hash(summary: &ConfirmationSummary) -> Result<String, ConfirmationHashError> {
    let value = serde_json::to_value(summary)?;
    let mut options = StableJsonOptions::default();
    options.ignore_object_keys.insert("ts".to_string());
    options.ignore_object_keys.insert("timestamp".to_string());
    options.ignore_object_keys.insert("created_at".to_string());
    options.ignore_object_keys.insert("updated_at".to_string());
    Ok(stable_hash_hex(&value, &options)?)
}

pub fn enrich_need_user_confirm_output(
    gate_input: &PolicyGateInput,
    gate_output: &PolicyGateOutput,
) -> Result<PolicyGateOutput, ConfirmationHashError> {
    let Some(summary) = build_confirmation_summary(gate_input, gate_output) else {
        return Ok(gate_output.clone());
    };
    let hash = confirmation_hash(&summary)?;
    let mut details_object = summary.details.clone();
    details_object.insert(
        "confirmation_summary".to_string(),
        serde_json::to_value(&summary).unwrap_or(Value::Null),
    );
    details_object.insert("confirmation_hash".to_string(), Value::String(hash));

    match gate_output {
        PolicyGateOutput::NeedUserConfirm { reason, .. } => Ok(PolicyGateOutput::NeedUserConfirm {
            reason: reason.clone(),
            details: details_object.clone(),
        }),
        _ => Ok(gate_output.clone()),
    }
}

#[cfg(test)]
#[path = "confirm_hash_test.rs"]
mod tests;
