use ais_engine::{
    canonical_side_effect_status, is_pending_side_effect_status, CheckpointApprovalLedgerEntry,
    CheckpointSideEffectRecord, EngineEventRecord, EngineEventType,
    SIDE_EFFECT_RECORD_SCHEMA_0_1_0, SIDE_EFFECT_STATUS_CONFIRMED, SIDE_EFFECT_STATUS_REVERTED,
    SIDE_EFFECT_STATUS_SENT,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct RunnerCheckpointLedger {
    approval_index: BTreeMap<String, CheckpointApprovalLedgerEntry>,
    side_effect_index: BTreeMap<String, CheckpointSideEffectRecord>,
}

impl Default for RunnerCheckpointLedger {
    fn default() -> Self {
        Self {
            approval_index: BTreeMap::new(),
            side_effect_index: BTreeMap::new(),
        }
    }
}

impl RunnerCheckpointLedger {
    pub fn from_checkpoint(
        approvals: &[CheckpointApprovalLedgerEntry],
        side_effects: &[CheckpointSideEffectRecord],
    ) -> Self {
        let mut out = Self::default();
        for entry in approvals {
            out.approval_index
                .insert(approval_key(entry), entry.clone());
        }
        for record in side_effects {
            let mut normalized = record.clone();
            normalize_side_effect_status(&mut normalized);
            if !is_valid_side_effect_record(&normalized) {
                continue;
            }
            let key = side_effect_key(&normalized);
            out.side_effect_index.insert(key, normalized);
        }
        out
    }

    pub fn absorb_events(&mut self, events: &[EngineEventRecord]) {
        for record in events {
            if record.event.event_type == EngineEventType::SideEffectObserved {
                let Some(raw_record) = record.event.data.get("record") else {
                    continue;
                };
                let Ok(mut side_effect) =
                    serde_json::from_value::<CheckpointSideEffectRecord>(raw_record.clone())
                else {
                    continue;
                };
                if side_effect.schema.is_none() {
                    side_effect.schema = Some(SIDE_EFFECT_RECORD_SCHEMA_0_1_0.to_string());
                }
                if side_effect.observed_at.trim().is_empty() {
                    side_effect.observed_at = record.ts.clone();
                }
                normalize_side_effect_status(&mut side_effect);
                if is_valid_side_effect_record(&side_effect) {
                    let key = side_effect_key(&side_effect);
                    self.side_effect_index.insert(key, side_effect);
                }
                continue;
            }
            if record.event.event_type != EngineEventType::NeedUserConfirm {
                continue;
            }
            let Some(node_id) = record.event.node_id.as_deref() else {
                continue;
            };
            let confirmation_hash = record
                .event
                .data
                .get("details")
                .and_then(Value::as_object)
                .and_then(|details| details.get("confirmation_hash"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let reason_code = record
                .event
                .data
                .get("reason_code")
                .and_then(Value::as_str)
                .map(str::to_string);
            let entry = CheckpointApprovalLedgerEntry {
                node_id: node_id.to_string(),
                confirmation_hash,
                decision: "requested".to_string(),
                reason_code,
                decided_at: record.ts.clone(),
            };
            self.approval_index.insert(approval_key(&entry), entry);
        }
    }

    pub fn mark_approved_nodes(&mut self, node_ids: &[String], ts: &str) {
        let approved_set = node_ids.iter().collect::<BTreeSet<_>>();
        for entry in self.approval_index.values_mut() {
            if approved_set.contains(&entry.node_id) {
                entry.decision = "approve".to_string();
                entry.decided_at = ts.to_string();
            }
        }
        for node_id in approved_set {
            let key = format!("{node_id}:no_confirmation_hash:approve");
            self.approval_index
                .entry(key)
                .or_insert_with(|| CheckpointApprovalLedgerEntry {
                    node_id: node_id.to_string(),
                    confirmation_hash: None,
                    decision: "approve".to_string(),
                    reason_code: None,
                    decided_at: ts.to_string(),
                });
        }
    }

    pub fn reconcile_completed_from_confirmed_side_effects(
        &self,
        completed_node_ids: &mut Vec<String>,
    ) {
        let mut completed = completed_node_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<String>>();
        for effect in self.side_effect_index.values() {
            if effect.status != SIDE_EFFECT_STATUS_CONFIRMED {
                continue;
            }
            completed.insert(effect.node_id.clone());
        }
        *completed_node_ids = completed.into_iter().collect();
    }

    pub fn pending_side_effects(&self) -> Vec<CheckpointSideEffectRecord> {
        self.side_effect_index
            .values()
            .filter(|effect| is_pending_side_effect_status(effect.status.as_str()))
            .cloned()
            .collect()
    }

    pub fn upsert_side_effect(&mut self, record: CheckpointSideEffectRecord) {
        let mut normalized = record;
        normalize_side_effect_status(&mut normalized);
        if !is_valid_side_effect_record(&normalized) {
            return;
        }
        let key = side_effect_key(&normalized);
        self.side_effect_index.insert(key, normalized);
    }

    pub fn approvals(&self) -> Vec<CheckpointApprovalLedgerEntry> {
        self.approval_index.values().cloned().collect()
    }

    pub fn side_effects(&self) -> Vec<CheckpointSideEffectRecord> {
        self.side_effect_index.values().cloned().collect()
    }

    pub fn side_effect_lifecycle_summary(&self) -> Value {
        let mut status_counts = BTreeMap::<String, usize>::new();
        let mut execution_counts = BTreeMap::<String, BTreeMap<String, usize>>::new();
        for effect in self.side_effect_index.values() {
            let status = canonical_side_effect_status(effect.status.as_str()).to_string();
            *status_counts.entry(status.clone()).or_insert(0) += 1;
            let execution_key = effect
                .execution_type
                .as_ref()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let execution_entry = execution_counts.entry(execution_key).or_default();
            *execution_entry.entry(status).or_insert(0) += 1;
        }
        let sent = status_counts
            .get(SIDE_EFFECT_STATUS_SENT)
            .copied()
            .unwrap_or(0);
        let confirmed = status_counts
            .get(SIDE_EFFECT_STATUS_CONFIRMED)
            .copied()
            .unwrap_or(0);
        let reverted = status_counts
            .get(SIDE_EFFECT_STATUS_REVERTED)
            .copied()
            .unwrap_or(0);
        json!({
            "schema": "ais-side-effect-lifecycle/0.1.0",
            "counts": {
                "total": self.side_effect_index.len(),
                "sent": sent,
                "confirmed": confirmed,
                "reverted": reverted,
            },
            "status_counts": status_counts,
            "by_execution_type": execution_counts,
        })
    }
}

fn approval_key(entry: &CheckpointApprovalLedgerEntry) -> String {
    format!(
        "{}:{}:{}",
        entry.node_id,
        entry
            .confirmation_hash
            .as_deref()
            .unwrap_or("no_confirmation_hash"),
        entry.decision
    )
}

fn side_effect_key(entry: &CheckpointSideEffectRecord) -> String {
    entry.idempotency_key.clone()
}

fn is_valid_side_effect_record(entry: &CheckpointSideEffectRecord) -> bool {
    !entry.idempotency_key.trim().is_empty()
        && !entry.node_id.trim().is_empty()
        && !entry.effect_type.trim().is_empty()
        && entry
            .chain
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        && entry
            .execution_type
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        && !entry.status.trim().is_empty()
        && !entry.observed_at.trim().is_empty()
}

fn normalize_side_effect_status(entry: &mut CheckpointSideEffectRecord) {
    entry.status = canonical_side_effect_status(entry.status.as_str()).to_string();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ais_engine::{EngineEvent, EngineEventRecord};
    use serde_json::json;

    #[test]
    fn ledger_absorbs_confirm_and_side_effect_events() {
        let mut ledger = RunnerCheckpointLedger::default();
        let mut confirm_event = EngineEvent::new(EngineEventType::NeedUserConfirm);
        confirm_event.node_id = Some("swap-1".to_string());
        confirm_event
            .data
            .insert("details".to_string(), json!({"confirmation_hash":"0xabc"}));
        let mut side_effect_event = EngineEvent::new(EngineEventType::SideEffectObserved);
        side_effect_event.data.insert(
            "record".to_string(),
            json!({
                "schema":"ais-side-effect-record/0.1.0",
                "effect_type":"tx",
                "idempotency_key":"tx:swap-1:0xtx1",
                "node_id":"swap-1",
                "chain":"eip155:1",
                "execution_type":"evm_call",
                "status":"sent",
                "observed_at":"2026-02-23T00:00:02Z",
                "tx_hash":"0xtx1",
                "nonce":9
            }),
        );
        ledger.absorb_events(&[
            EngineEventRecord {
                schema: "ais-engine-event/0.0.3".to_string(),
                run_id: "run-1".to_string(),
                seq: 1,
                ts: "2026-02-23T00:00:00Z".to_string(),
                event: confirm_event,
            },
            EngineEventRecord {
                schema: "ais-engine-event/0.0.3".to_string(),
                run_id: "run-1".to_string(),
                seq: 2,
                ts: "2026-02-23T00:00:02Z".to_string(),
                event: side_effect_event,
            },
        ]);
        ledger.mark_approved_nodes(&["swap-1".to_string()], "2026-02-23T00:00:01Z");
        assert_eq!(ledger.approvals().len(), 2);
        assert_eq!(ledger.side_effects().len(), 1);
        let effect = ledger
            .side_effects()
            .into_iter()
            .next()
            .expect("side effect");
        assert_eq!(effect.execution_type.as_deref(), Some("evm_call"));
        assert_eq!(
            effect.schema.as_deref(),
            Some("ais-side-effect-record/0.1.0")
        );
    }

    #[test]
    fn ledger_absorbs_side_effect_observed_event() {
        let mut ledger = RunnerCheckpointLedger::default();
        let mut event = EngineEvent::new(EngineEventType::SideEffectObserved);
        event.data.insert(
            "record".to_string(),
            json!({
                "schema":"ais-side-effect-record/0.1.0",
                "effect_type":"tx",
                "idempotency_key":"tx:swap-1:0xabc",
                "node_id":"swap-1",
                "chain":"eip155:1",
                "execution_type":"evm_call",
                "status":"sent",
                "observed_at":"2026-02-24T00:00:00Z",
                "tx_hash":"0xabc"
            }),
        );
        ledger.absorb_events(&[EngineEventRecord {
            schema: "ais-engine-event/0.0.3".to_string(),
            run_id: "run-1".to_string(),
            seq: 2,
            ts: "2026-02-24T00:00:00Z".to_string(),
            event,
        }]);
        let effects = ledger.side_effects();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].execution_type.as_deref(), Some("evm_call"));
        assert_eq!(effects[0].chain.as_deref(), Some("eip155:1"));
    }

    #[test]
    fn ledger_drops_side_effect_without_idempotency_key() {
        let mut ledger = RunnerCheckpointLedger::default();
        let mut event = EngineEvent::new(EngineEventType::SideEffectObserved);
        event.data.insert(
            "record".to_string(),
            json!({
                "schema":"ais-side-effect-record/0.1.0",
                "effect_type":"tx",
                "idempotency_key":"",
                "node_id":"swap-1",
                "chain":"eip155:1",
                "execution_type":"evm_call",
                "status":"sent",
                "observed_at":"2026-02-24T00:00:00Z",
                "tx_hash":"0xabc"
            }),
        );
        ledger.absorb_events(&[EngineEventRecord {
            schema: "ais-engine-event/0.0.3".to_string(),
            run_id: "run-1".to_string(),
            seq: 2,
            ts: "2026-02-24T00:00:00Z".to_string(),
            event,
        }]);
        assert!(ledger.side_effects().is_empty());
    }

    #[test]
    fn ledger_normalizes_failed_status_to_reverted() {
        let mut ledger = RunnerCheckpointLedger::default();
        let mut event = EngineEvent::new(EngineEventType::SideEffectObserved);
        event.data.insert(
            "record".to_string(),
            json!({
                "schema":"ais-side-effect-record/0.1.0",
                "effect_type":"tx",
                "idempotency_key":"tx:swap-1:0xabc",
                "node_id":"swap-1",
                "chain":"eip155:1",
                "execution_type":"evm_call",
                "status":"failed",
                "observed_at":"2026-02-24T00:00:00Z",
                "tx_hash":"0xabc"
            }),
        );
        ledger.absorb_events(&[EngineEventRecord {
            schema: "ais-engine-event/0.0.3".to_string(),
            run_id: "run-1".to_string(),
            seq: 2,
            ts: "2026-02-24T00:00:00Z".to_string(),
            event,
        }]);
        let effects = ledger.side_effects();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].status, "reverted");
    }

    #[test]
    fn lifecycle_summary_reports_sent_confirmed_reverted_counts() {
        let ledger = RunnerCheckpointLedger::from_checkpoint(
            &[],
            &[
                CheckpointSideEffectRecord {
                    schema: Some("ais-side-effect-record/0.1.0".to_string()),
                    idempotency_key: "tx:n1:0xa".to_string(),
                    node_id: "n1".to_string(),
                    effect_type: "tx".to_string(),
                    chain: Some("eip155:1".to_string()),
                    execution_type: Some("evm_call".to_string()),
                    tx_hash: Some("0xa".to_string()),
                    nonce: None,
                    provider_ref: None,
                    reason_code: None,
                    details: None,
                    status: "sent".to_string(),
                    observed_at: "2026-02-24T00:00:00Z".to_string(),
                },
                CheckpointSideEffectRecord {
                    schema: Some("ais-side-effect-record/0.1.0".to_string()),
                    idempotency_key: "tx:n2:0xb".to_string(),
                    node_id: "n2".to_string(),
                    effect_type: "tx".to_string(),
                    chain: Some("eip155:1".to_string()),
                    execution_type: Some("evm_call".to_string()),
                    tx_hash: Some("0xb".to_string()),
                    nonce: None,
                    provider_ref: None,
                    reason_code: None,
                    details: None,
                    status: "confirmed".to_string(),
                    observed_at: "2026-02-24T00:00:00Z".to_string(),
                },
                CheckpointSideEffectRecord {
                    schema: Some("ais-side-effect-record/0.1.0".to_string()),
                    idempotency_key: "tx:n3:0xc".to_string(),
                    node_id: "n3".to_string(),
                    effect_type: "tx".to_string(),
                    chain: Some("eip155:1".to_string()),
                    execution_type: Some("evm_call".to_string()),
                    tx_hash: Some("0xc".to_string()),
                    nonce: None,
                    provider_ref: None,
                    reason_code: None,
                    details: None,
                    status: "failed".to_string(),
                    observed_at: "2026-02-24T00:00:00Z".to_string(),
                },
            ],
        );
        let summary = ledger.side_effect_lifecycle_summary();
        assert_eq!(
            summary.pointer("/counts/sent").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary.pointer("/counts/confirmed").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            summary.pointer("/counts/reverted").and_then(Value::as_u64),
            Some(1)
        );
    }
}
