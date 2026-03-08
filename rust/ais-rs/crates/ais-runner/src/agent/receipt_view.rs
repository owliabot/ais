use crate::checkpoint_ledger::RunnerCheckpointLedger;
use ais_engine::{EngineEventRecord, EngineRunStatus, EngineRunnerState};
use ais_sdk::documents::PlanSketchSegment;
use serde_json::Value;
use std::collections::BTreeSet;

use super::todos::TodoReceipt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptSideEffectView {
    pub node_id: String,
    pub effect_type: String,
    pub chain: Option<String>,
    pub execution_type: Option<String>,
    pub status: String,
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptView {
    pub node_ids: Vec<String>,
    pub side_effects: Vec<ReceiptSideEffectView>,
    pub tx_hashes: Vec<String>,
}

impl ReceiptView {
    pub(crate) fn from_ledger(
        node_ids: &[String],
        checkpoint_ledger: &RunnerCheckpointLedger,
    ) -> Self {
        let node_set = node_ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let tx_hashes = checkpoint_ledger.tx_hashes_for_nodes(node_ids);
        let mut side_effects = checkpoint_ledger
            .side_effects()
            .into_iter()
            .filter(|effect| node_set.contains(effect.node_id.as_str()))
            .map(|effect| {
                let tx_hash = effect
                    .tx_hash
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                ReceiptSideEffectView {
                    node_id: effect.node_id,
                    effect_type: effect.effect_type,
                    chain: effect.chain,
                    execution_type: effect.execution_type,
                    status: effect.status,
                    tx_hash,
                }
            })
            .collect::<Vec<_>>();
        side_effects.sort_by(|left, right| {
            left.node_id
                .cmp(&right.node_id)
                .then(left.effect_type.cmp(&right.effect_type))
                .then(left.status.cmp(&right.status))
        });
        Self {
            node_ids: node_ids.to_vec(),
            side_effects,
            tx_hashes,
        }
    }

    pub(crate) fn apply_to_receipt(&self, receipt: &mut TodoReceipt) {
        receipt.tx_hashes = self.tx_hashes.clone();
    }

    pub(crate) fn project_runtime_receipt(&self, runtime: &mut Value) {
        let tx_hashes_value = self
            .tx_hashes
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>();
        if let Some(current_todo) = runtime.pointer_mut("/agent/todo_progress/current_todo") {
            overwrite_todo_receipt_tx_hashes(
                current_todo,
                self.node_ids.as_slice(),
                tx_hashes_value.as_slice(),
            );
        }
        if let Some(todos) = runtime
            .pointer_mut("/agent/todo_progress/todos")
            .and_then(Value::as_array_mut)
        {
            for todo in todos {
                overwrite_todo_receipt_tx_hashes(
                    todo,
                    self.node_ids.as_slice(),
                    tx_hashes_value.as_slice(),
                );
            }
        }
    }
}

pub(crate) fn build_segment_todo_receipt(
    todo_id: &str,
    segment: &PlanSketchSegment,
    status: EngineRunStatus,
    state: &EngineRunnerState,
    round_events: &[EngineEventRecord],
    checkpoint_ledger: Option<&RunnerCheckpointLedger>,
) -> TodoReceipt {
    let node_ids = segment
        .steps
        .iter()
        .map(|step| format!("{}/{}", segment.segment_id, step.id))
        .collect::<Vec<_>>();
    let completed_node_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let completed_node_ids = node_ids
        .iter()
        .filter(|node_id| completed_node_set.contains((*node_id).as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let event_types = round_events
        .iter()
        .map(event_type_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let mut receipt = TodoReceipt {
        schema: "ais-agent-todo-receipt/0.0.1".to_string(),
        todo_id: todo_id.to_string(),
        segment_id: segment.segment_id.clone(),
        status: run_status_name(status).to_string(),
        paused_reason: state.paused_reason.clone(),
        node_ids,
        completed_node_ids,
        tx_hashes: Vec::new(),
        event_types,
        event_count: round_events.len() as u64,
    };
    if let Some(checkpoint_ledger) = checkpoint_ledger {
        let ledger_view = ReceiptView::from_ledger(receipt.node_ids.as_slice(), checkpoint_ledger);
        ledger_view.apply_to_receipt(&mut receipt);
    }
    receipt
}

pub(crate) fn project_todo_progress_receipts_from_ledger(
    runtime: &mut Value,
    checkpoint_ledger: &RunnerCheckpointLedger,
) {
    normalize_todo_progress_receipt_shapes(runtime);
    let receipt_views = collect_todo_progress_receipt_views(runtime, checkpoint_ledger);
    if receipt_views.is_empty() {
        return;
    }
    for receipt_view in receipt_views {
        receipt_view.project_runtime_receipt(runtime);
    }
}

pub(crate) fn collect_todo_progress_receipt_views(
    runtime: &Value,
    checkpoint_ledger: &RunnerCheckpointLedger,
) -> Vec<ReceiptView> {
    let mut out = BTreeSet::<Vec<String>>::new();
    if let Some(current) = runtime.pointer("/agent/todo_progress/current_todo") {
        if let Some(node_ids) = todo_receipt_node_ids(current) {
            out.insert(node_ids);
        }
    }
    if let Some(todos) = runtime
        .pointer("/agent/todo_progress/todos")
        .and_then(Value::as_array)
    {
        for todo in todos {
            if let Some(node_ids) = todo_receipt_node_ids(todo) {
                out.insert(node_ids);
            }
        }
    }
    out.into_iter()
        .map(|node_ids| ReceiptView::from_ledger(node_ids.as_slice(), checkpoint_ledger))
        .collect()
}

pub(crate) fn normalize_todo_progress_receipt_shapes(runtime: &mut Value) {
    if let Some(todo_progress) = runtime.pointer_mut("/agent/todo_progress") {
        normalize_todo_progress_value(todo_progress);
    }
}

pub(crate) fn normalize_todo_progress_value(todo_progress: &mut Value) {
    if let Some(current) = todo_progress.pointer_mut("/current_todo") {
        normalize_todo_receipt_shape(current);
    }
    if let Some(todos) = todo_progress
        .pointer_mut("/todos")
        .and_then(Value::as_array_mut)
    {
        for todo in todos {
            normalize_todo_receipt_shape(todo);
        }
    }
}

fn todo_receipt_node_ids(todo: &Value) -> Option<Vec<String>> {
    let node_ids = todo
        .pointer("/receipt/node_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if node_ids.is_empty() {
        return None;
    }
    Some(node_ids)
}

fn normalize_todo_receipt_shape(todo: &mut Value) {
    let Some(receipt_obj) = todo.pointer_mut("/receipt").and_then(Value::as_object_mut) else {
        return;
    };
    let has_node_ids = receipt_obj
        .get("node_ids")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .any(|value| !value.is_empty())
        });
    let tx_hashes = match receipt_obj.get("tx_hashes") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect::<Vec<_>>(),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![Value::String(trimmed.to_string())]
            }
        }
        Some(Value::Null) => Vec::new(),
        Some(_) => Vec::new(),
        None => return,
    };
    let normalized = if has_node_ids { tx_hashes } else { Vec::new() };
    receipt_obj.insert("tx_hashes".to_string(), Value::Array(normalized));
}

fn overwrite_todo_receipt_tx_hashes(todo: &mut Value, node_ids: &[String], tx_hashes: &[Value]) {
    let Some(todo_obj) = todo.as_object_mut() else {
        return;
    };
    let Some(receipt_obj) = todo_obj.get_mut("receipt").and_then(Value::as_object_mut) else {
        return;
    };
    let receipt_node_ids = receipt_obj
        .get("node_ids")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let expected_node_ids = node_ids.iter().cloned().collect::<BTreeSet<_>>();
    if receipt_node_ids != expected_node_ids {
        return;
    }
    receipt_obj.insert("tx_hashes".to_string(), Value::Array(tx_hashes.to_vec()));
}

fn run_status_name(status: EngineRunStatus) -> &'static str {
    match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    }
}

fn event_type_name(record: &EngineEventRecord) -> String {
    serde_json::to_value(record.event.event_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{:?}", record.event.event_type).to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType};

    #[test]
    fn receipt_view_collects_chain_agnostic_side_effects_and_tx_hashes() {
        let mut tx_event = EngineEvent::new(EngineEventType::SideEffectObserved);
        tx_event.data.insert(
            "record".to_string(),
            serde_json::json!({
                "schema":"ais-side-effect-record/0.1.0",
                "idempotency_key":"tx:n1:0x1",
                "node_id":"n1",
                "effect_type":"tx",
                "chain":"solana:mainnet",
                "execution_type":"solana_send",
                "tx_hash":"0x1",
                "status":"sent",
                "observed_at":"1970-01-01T00:00:00Z"
            }),
        );
        let mut message_event = EngineEvent::new(EngineEventType::SideEffectObserved);
        message_event.data.insert(
            "record".to_string(),
            serde_json::json!({
                "schema":"ais-side-effect-record/0.1.0",
                "idempotency_key":"msg:n1:1",
                "node_id":"n1",
                "effect_type":"message",
                "chain":"solana:mainnet",
                "execution_type":"notify",
                "status":"sent",
                "observed_at":"1970-01-01T00:00:01Z"
            }),
        );
        let mut ledger = RunnerCheckpointLedger::default();
        ledger.absorb_events(&[
            EngineEventRecord::new("run", 1, "1970-01-01T00:00:00Z", tx_event),
            EngineEventRecord::new("run", 2, "1970-01-01T00:00:01Z", message_event),
        ]);

        let view = ReceiptView::from_ledger(&["n1".to_string()], &ledger);
        assert_eq!(view.tx_hashes, vec!["0x1".to_string()]);
        assert_eq!(view.side_effects.len(), 2);
        assert_eq!(
            view.side_effects[0].chain.as_deref(),
            Some("solana:mainnet")
        );
    }

    #[test]
    fn project_runtime_receipt_clears_stale_tx_hashes_when_ledger_has_none() {
        let mut runtime = serde_json::json!({
            "agent": {
                "todo_progress": {
                    "current_todo": {
                        "receipt": {
                            "node_ids":["seg_1/a1"],
                            "tx_hashes":["0xstale"]
                        }
                    },
                    "todos":[
                        {
                            "receipt": {
                                "node_ids":["seg_1/a1"],
                                "tx_hashes":["0xstale"]
                            }
                        }
                    ]
                }
            }
        });

        ReceiptView {
            node_ids: vec!["seg_1/a1".to_string()],
            side_effects: Vec::new(),
            tx_hashes: Vec::new(),
        }
        .project_runtime_receipt(&mut runtime);

        assert_eq!(
            runtime.pointer("/agent/todo_progress/current_todo/receipt/tx_hashes"),
            Some(&serde_json::json!([]))
        );
        assert_eq!(
            runtime.pointer("/agent/todo_progress/todos/0/receipt/tx_hashes"),
            Some(&serde_json::json!([]))
        );
    }

    #[test]
    fn normalize_todo_receipt_shape_clears_tx_hashes_when_node_ids_missing() {
        let mut todo = serde_json::json!({
            "receipt": {
                "tx_hashes": ["0xstale"]
            }
        });

        normalize_todo_receipt_shape(&mut todo);

        assert_eq!(
            todo.pointer("/receipt/tx_hashes"),
            Some(&serde_json::json!([]))
        );
    }
}
