use crate::trace::{redact_value, TraceRedactOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const CHECKPOINT_SCHEMA_0_0_1: &str = "ais-checkpoint/0.0.1";
pub const SIDE_EFFECT_RECORD_SCHEMA_0_1_0: &str = "ais-side-effect-record/0.1.0";
pub const SIDE_EFFECT_STATUS_PREPARED: &str = "prepared";
pub const SIDE_EFFECT_STATUS_SENT: &str = "sent";
pub const SIDE_EFFECT_STATUS_CONFIRMED: &str = "confirmed";
pub const SIDE_EFFECT_STATUS_REVERTED: &str = "reverted";
pub const SIDE_EFFECT_STATUS_UNKNOWN: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CheckpointEngineState {
    #[serde(default)]
    pub completed_node_ids: Vec<String>,
    #[serde(default)]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub seen_command_ids: Vec<String>,
    #[serde(default)]
    pub pending_retries: Map<String, Value>,
    #[serde(default)]
    pub plan_epoch: u64,
    #[serde(default)]
    pub plan_hash_history: Vec<String>,
}

impl CheckpointEngineState {
    pub fn normalize(&mut self) {
        self.completed_node_ids = dedup_sort_strings(std::mem::take(&mut self.completed_node_ids));
        self.seen_command_ids = dedup_sort_strings(std::mem::take(&mut self.seen_command_ids));
        if self.plan_hash_history.is_empty() {
            return;
        }
        let mut deduped = Vec::<String>::new();
        for hash in std::mem::take(&mut self.plan_hash_history) {
            if deduped.last() != Some(&hash) {
                deduped.push(hash);
            }
        }
        self.plan_hash_history = deduped;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointDocument {
    pub schema: String,
    pub run_id: String,
    pub plan_hash: String,
    pub engine_state: CheckpointEngineState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals_ledger: Vec<CheckpointApprovalLedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub side_effects: Vec<CheckpointSideEffectRecord>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extensions: Map<String, Value>,
}

impl CheckpointDocument {
    pub fn new(
        run_id: impl Into<String>,
        plan_hash: impl Into<String>,
        mut engine_state: CheckpointEngineState,
        runtime_snapshot: Option<Value>,
        plan_snapshot: Option<Value>,
    ) -> Self {
        engine_state.normalize();
        Self {
            schema: CHECKPOINT_SCHEMA_0_0_1.to_string(),
            run_id: run_id.into(),
            plan_hash: plan_hash.into(),
            engine_state,
            runtime_snapshot,
            plan_snapshot,
            approvals_ledger: vec![],
            side_effects: vec![],
            extensions: Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointApprovalLedgerEntry {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_hash: Option<String>,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub decided_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointSideEffectRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub idempotency_key: String,
    pub node_id: String,
    pub effect_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub status: String,
    pub observed_at: String,
}

pub fn create_checkpoint_document(
    run_id: impl Into<String>,
    plan_hash: impl Into<String>,
    engine_state: CheckpointEngineState,
    runtime_snapshot: Option<Value>,
    plan_snapshot: Option<Value>,
    redact_options: Option<&TraceRedactOptions>,
) -> CheckpointDocument {
    let runtime_snapshot = runtime_snapshot.map(|mut value| {
        if let Some(options) = redact_options {
            redact_value(&mut value, options);
        }
        value
    });
    CheckpointDocument::new(
        run_id,
        plan_hash,
        engine_state,
        runtime_snapshot,
        plan_snapshot,
    )
}

pub fn encode_checkpoint_json(document: &CheckpointDocument) -> serde_json::Result<String> {
    serde_json::to_string_pretty(document)
}

pub fn decode_checkpoint_json(input: &str) -> serde_json::Result<CheckpointDocument> {
    let mut document = serde_json::from_str::<CheckpointDocument>(input)?;
    normalize_checkpoint_ledgers(&mut document);
    Ok(document)
}

pub fn canonical_side_effect_status(status: &str) -> &'static str {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "prepared" | "signed" => SIDE_EFFECT_STATUS_PREPARED,
        "sent" | "submitted" | "broadcast" | "pending" => SIDE_EFFECT_STATUS_SENT,
        "confirmed" | "finalized" | "success" | "succeeded" | "ok" => SIDE_EFFECT_STATUS_CONFIRMED,
        "reverted" | "failed" | "error" | "dropped" => SIDE_EFFECT_STATUS_REVERTED,
        _ => SIDE_EFFECT_STATUS_UNKNOWN,
    }
}

pub fn is_pending_side_effect_status(status: &str) -> bool {
    canonical_side_effect_status(status) == SIDE_EFFECT_STATUS_SENT
}

pub fn is_terminal_side_effect_status(status: &str) -> bool {
    matches!(
        canonical_side_effect_status(status),
        SIDE_EFFECT_STATUS_CONFIRMED | SIDE_EFFECT_STATUS_REVERTED
    )
}

fn dedup_sort_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_checkpoint_ledgers(document: &mut CheckpointDocument) {
    if !document.approvals_ledger.is_empty() {
        let mut by_key = BTreeMap::<String, CheckpointApprovalLedgerEntry>::new();
        for entry in std::mem::take(&mut document.approvals_ledger) {
            let key = format!(
                "{}:{}:{}",
                entry.node_id,
                entry
                    .confirmation_hash
                    .as_deref()
                    .unwrap_or("no_confirmation_hash"),
                entry.decision
            );
            by_key.insert(key, entry);
        }
        document.approvals_ledger = by_key.into_values().collect();
    }

    if !document.side_effects.is_empty() {
        let mut by_key = BTreeMap::<String, CheckpointSideEffectRecord>::new();
        for mut entry in std::mem::take(&mut document.side_effects) {
            if entry.idempotency_key.trim().is_empty() {
                continue;
            }
            entry.status = canonical_side_effect_status(entry.status.as_str()).to_string();
            let key = entry.idempotency_key.clone();
            by_key.insert(key, entry);
        }
        document.side_effects = by_key.into_values().collect();
    }
}

#[cfg(test)]
#[path = "types_test.rs"]
mod tests;
