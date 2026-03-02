use super::{TodoBoard, TodoReceipt, TodoSpec, TodoStatus};
use serde_json::json;

#[test]
fn todo_board_transitions_current_item() {
    let mut board = TodoBoard::bootstrap("transfer 10 usdc");
    assert_eq!(
        board.current().map(|item| item.status),
        Some(TodoStatus::Todo)
    );

    board.mark_current_in_progress(Some("query balances"), "seg_1");
    let current = board.current().expect("current todo");
    assert_eq!(current.status, TodoStatus::InProgress);
    assert_eq!(current.segment_id.as_deref(), Some("seg_1"));
    assert_eq!(current.title, "query balances");

    board.mark_current_blocked("missing_required_input");
    assert_eq!(
        board
            .current()
            .and_then(|item| item.blocked_reason.as_deref()),
        Some("missing_required_input")
    );

    board.mark_current_todo();
    assert_eq!(
        board.current().map(|item| item.status),
        Some(TodoStatus::Todo)
    );

    board.mark_current_done();
    assert_eq!(board.current(), None);

    board.open_follow_up_todo();
    let current = board.current().expect("follow-up todo");
    assert_eq!(current.id, "todo_2");
    assert_eq!(current.status, TodoStatus::Todo);
}

#[test]
fn todo_board_can_restore_from_runtime_payload() {
    let runtime = json!({
        "agent": {
            "todo_progress": {
                "todos": [{
                    "id": "todo_7",
                    "title": "resume",
                    "status": "in_progress",
                    "acceptance": [],
                    "required_facts": [],
                    "produced_facts": [],
                    "segment_id": "seg_7"
                }],
                "next_seq": 8
            }
        }
    });
    let board = TodoBoard::restore_or_bootstrap(&runtime, "ignored");
    let current = board.current().expect("current todo");
    assert_eq!(current.id, "todo_7");
    assert_eq!(current.status, TodoStatus::InProgress);
    assert_eq!(current.segment_id.as_deref(), Some("seg_7"));
    let value = board.to_runtime_value();
    assert_eq!(value.pointer("/next_seq"), Some(&json!(8)));
}

#[test]
fn todo_board_replace_from_specs_normalizes_and_dedupes() {
    let mut board = TodoBoard::bootstrap("transfer 10 usdc");
    board.replace_from_specs(
        "transfer 10 usdc",
        &[
            TodoSpec {
                title: " Query allowance ".to_string(),
                required_facts: vec![" owner ".to_string(), "owner".to_string()],
                produced_facts: vec![" allowance ".to_string()],
                acceptance: vec!["".to_string(), "allowance_ready".to_string()],
            },
            TodoSpec {
                title: "query allowance".to_string(),
                required_facts: vec!["ignored".to_string()],
                produced_facts: vec![],
                acceptance: vec![],
            },
            TodoSpec {
                title: "Execute transfer".to_string(),
                required_facts: vec!["allowance".to_string()],
                produced_facts: vec!["tx_hash".to_string()],
                acceptance: vec![],
            },
        ],
    );
    let runtime = board.to_runtime_value();
    assert_eq!(runtime.pointer("/todos/0/id"), Some(&json!("todo_1")));
    assert_eq!(
        runtime.pointer("/todos/0/title"),
        Some(&json!("Query allowance"))
    );
    assert_eq!(
        runtime.pointer("/todos/0/required_facts"),
        Some(&json!(["owner"]))
    );
    assert_eq!(
        runtime.pointer("/todos/0/acceptance"),
        Some(&json!(["allowance_ready"]))
    );
    assert_eq!(runtime.pointer("/todos/1/id"), Some(&json!("todo_2")));
    assert_eq!(
        runtime.pointer("/todos/1/title"),
        Some(&json!("Execute transfer"))
    );
    assert_eq!(runtime.pointer("/next_seq"), Some(&json!(3)));
}

#[test]
fn todo_board_records_receipt_by_todo_id() {
    let mut board = TodoBoard::bootstrap("transfer 10 usdc");
    let todo_id = board.current_todo_id().expect("todo id").to_string();
    let recorded = board.record_receipt_for_todo(
        todo_id.as_str(),
        TodoReceipt {
            schema: "ais-agent-todo-receipt/0.0.1".to_string(),
            todo_id: todo_id.clone(),
            segment_id: "seg_1".to_string(),
            status: "completed".to_string(),
            paused_reason: None,
            node_ids: vec!["seg_1/q1".to_string()],
            completed_node_ids: vec!["seg_1/q1".to_string()],
            tx_hashes: vec!["0xabc".to_string()],
            event_types: vec!["query_result".to_string()],
            event_count: 2,
        },
    );
    assert!(recorded);
    let runtime = board.to_runtime_value();
    assert_eq!(
        runtime.pointer("/current_todo/receipt/segment_id"),
        Some(&json!("seg_1"))
    );
    assert_eq!(
        runtime.pointer("/current_todo/receipt/event_count"),
        Some(&json!(2))
    );
}

#[test]
fn intent_acceptance_complete_reads_input_and_intent_context_from_state_summary() {
    let mut board = TodoBoard::bootstrap("transfer 10 usdc");
    board.replace_from_specs(
        "transfer 10 usdc",
        &[TodoSpec {
            title: "Execute transfer".to_string(),
            required_facts: vec![],
            produced_facts: vec![],
            acceptance: vec!["owner".to_string(), "tx.hash".to_string()],
        }],
    );
    board.mark_current_done();
    let state_summary = json!({
        "input_registry": {
            "known_refs": ["inputs.owner"]
        },
        "intent_context": {
            "facts": {
                "tx": {
                    "hash": "0xabc"
                }
            }
        }
    });
    assert!(board.intent_acceptance_complete(Some(&state_summary)));
}
