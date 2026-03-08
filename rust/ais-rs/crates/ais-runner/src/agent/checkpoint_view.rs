use crate::agent::receipt_view;
use crate::checkpoint_ledger::RunnerCheckpointLedger;
use ais_engine::EngineRunnerState;
use serde_json::{Map, Value};

pub(super) struct CheckpointView {
    normalized_runtime: Value,
}

impl CheckpointView {
    pub(super) fn from_state(
        state: &EngineRunnerState,
        checkpoint_ledger: &RunnerCheckpointLedger,
    ) -> Self {
        let mut runtime = state.runtime.clone();
        receipt_view::normalize_todo_progress_receipt_shapes(&mut runtime);
        normalize_todo_progress_receipt_tx_hashes_from_ledger(&mut runtime, checkpoint_ledger);
        archive_stale_missing_input_autofill(&mut runtime);
        Self {
            normalized_runtime: runtime,
        }
    }

    pub(super) fn runtime(&self) -> &Value {
        &self.normalized_runtime
    }
}

fn normalize_todo_progress_receipt_tx_hashes_from_ledger(
    runtime: &mut Value,
    checkpoint_ledger: &RunnerCheckpointLedger,
) {
    for receipt_view in
        receipt_view::collect_todo_progress_receipt_views(runtime, checkpoint_ledger)
    {
        receipt_view.project_runtime_receipt(runtime);
    }
}

fn archive_stale_missing_input_autofill(runtime: &mut Value) {
    let grounding_ready = runtime
        .pointer("/agent/intent_grounding/resolution_state")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "ready")
        || runtime
            .pointer("/agent/intent_grounding/ready_for_todos")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if !grounding_ready {
        return;
    }
    let Some(agent_obj) = runtime.pointer_mut("/agent").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(stale) = agent_obj.remove("missing_input_autofill") else {
        return;
    };
    let history = agent_obj
        .entry("recovery_history".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !history.is_object() {
        *history = Value::Object(Map::new());
    }
    if let Some(history_obj) = history.as_object_mut() {
        history_obj.insert("missing_input_autofill".to_string(), stale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType};

    #[test]
    fn checkpoint_view_projects_todo_receipt_tx_hashes_from_ledger() {
        let state = EngineRunnerState {
            runtime: serde_json::json!({
                "agent": {
                    "todo_progress": {
                        "current_todo": {
                            "id":"todo_1",
                            "receipt": {
                                "node_ids":["seg_1/native_send","seg_1/erc20_send"],
                                "tx_hashes":[]
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };
        let mut native_event = EngineEvent::new(EngineEventType::SideEffectObserved);
        native_event.data = serde_json::Map::from_iter([(
            "record".to_string(),
            serde_json::json!({
                "schema":"ais-side-effect-record/0.1.0",
                "idempotency_key":"tx:seg_1/native_send:0x1",
                "node_id":"seg_1/native_send",
                "effect_type":"tx",
                "chain":"eip155:1",
                "execution_type":"evm_send",
                "tx_hash":"0x1",
                "status":"sent",
                "observed_at":"1970-01-01T00:00:00Z"
            }),
        )]);
        let mut token_event = EngineEvent::new(EngineEventType::SideEffectObserved);
        token_event.data = serde_json::Map::from_iter([(
            "record".to_string(),
            serde_json::json!({
                "schema":"ais-side-effect-record/0.1.0",
                "idempotency_key":"tx:seg_1/erc20_send:0x2",
                "node_id":"seg_1/erc20_send",
                "effect_type":"tx",
                "chain":"eip155:1",
                "execution_type":"erc20_send",
                "tx_hash":"0x2",
                "status":"sent",
                "observed_at":"1970-01-01T00:00:01Z"
            }),
        )]);
        let mut ledger = RunnerCheckpointLedger::default();
        ledger.absorb_events(&[
            EngineEventRecord::new("run", 1, "1970-01-01T00:00:00Z", native_event),
            EngineEventRecord::new("run", 2, "1970-01-01T00:00:01Z", token_event),
        ]);

        let view = CheckpointView::from_state(&state, &ledger);
        assert_eq!(
            view.runtime()
                .pointer("/agent/todo_progress/current_todo/receipt/tx_hashes"),
            Some(&serde_json::json!(["0x1", "0x2"]))
        );
    }

    #[test]
    fn checkpoint_view_archives_stale_missing_input_autofill_when_grounding_ready() {
        let state = EngineRunnerState {
            runtime: serde_json::json!({
                "agent": {
                    "intent_grounding": {
                        "resolution_state":"ready",
                        "ready_for_todos": true
                    },
                    "missing_input_autofill": {
                        "reason":"query_exec_failed"
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = CheckpointView::from_state(&state, &RunnerCheckpointLedger::default());
        assert_eq!(
            view.runtime().pointer("/agent/missing_input_autofill"),
            None
        );
        assert_eq!(
            view.runtime()
                .pointer("/agent/recovery_history/missing_input_autofill/reason"),
            Some(&serde_json::json!("query_exec_failed"))
        );
    }

    #[test]
    fn checkpoint_view_normalizes_legacy_todo_receipt_tx_hash_shape_without_ledger() {
        let state = EngineRunnerState {
            runtime: serde_json::json!({
                "agent": {
                    "todo_progress": {
                        "current_todo": {
                            "id":"todo_1",
                            "receipt": {
                                "node_ids":["seg_1/native_send"],
                                "tx_hashes":"0xabc"
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = CheckpointView::from_state(&state, &RunnerCheckpointLedger::default());
        assert_eq!(
            view.runtime()
                .pointer("/agent/todo_progress/current_todo/receipt/tx_hashes"),
            Some(&serde_json::json!([]))
        );
    }

    #[test]
    fn checkpoint_view_clears_malformed_receipt_tx_hashes_without_node_ids() {
        let state = EngineRunnerState {
            runtime: serde_json::json!({
                "agent": {
                    "todo_progress": {
                        "current_todo": {
                            "id":"todo_1",
                            "receipt": {
                                "tx_hashes":["0xabc"]
                            }
                        }
                    }
                }
            }),
            ..EngineRunnerState::default()
        };

        let view = CheckpointView::from_state(&state, &RunnerCheckpointLedger::default());
        assert_eq!(
            view.runtime()
                .pointer("/agent/todo_progress/current_todo/receipt/tx_hashes"),
            Some(&serde_json::json!([]))
        );
    }
}
