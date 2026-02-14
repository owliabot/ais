use super::{replay_from_checkpoint, replay_trace_events, replay_trace_jsonl, ReplayOptions, ReplayStatus};
use crate::checkpoint::{create_checkpoint_document, decode_checkpoint_json, CheckpointDocument, CheckpointEngineState};
use crate::events::{
    encode_event_jsonl_line, EngineEvent, EngineEventRecord, EngineEventStream, EngineEventType,
};
use crate::executor::{Executor, ExecutorOutput, RouterExecutor};
use crate::solver::DefaultSolver;
use ais_sdk::PlanDocument;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

struct MockExecutor;

impl Executor for MockExecutor {
    fn execute(&self, node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
        let id = node
            .as_object()
            .and_then(|obj| obj.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        Ok(ExecutorOutput {
            result: json!({"done": true, "node_id": id}),
            writes: Map::new(),
        })
    }
}

fn sample_plan() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![
            json!({"id":"n1","chain":"eip155:1","execution":{"type":"evm_read","method":"balanceOf"},"writes":[]}),
            json!({"id":"n2","chain":"eip155:1","deps":["n1"],"execution":{"type":"evm_read","method":"balanceOf"},"writes":[]}),
        ],
        extensions: Map::new(),
    }
}

#[test]
fn replay_trace_events_stops_at_until_node() {
    let mut stream = EngineEventStream::new("run-trace");
    let mut e1 = EngineEvent::new(EngineEventType::NodeReady);
    e1.node_id = Some("n1".to_string());
    let mut e2 = EngineEvent::new(EngineEventType::NodeReady);
    e2.node_id = Some("n2".to_string());
    let mut e3 = EngineEvent::new(EngineEventType::NodeReady);
    e3.node_id = Some("n3".to_string());
    let events = vec![
        stream.next_record("2026-02-13T00:00:00Z", e1),
        stream.next_record("2026-02-13T00:00:01Z", e2),
        stream.next_record("2026-02-13T00:00:02Z", e3),
    ];

    let result = replay_trace_events(
        &events,
        &ReplayOptions {
            until_node: Some("n2".to_string()),
            max_steps: 8,
        },
    );
    assert_eq!(result.status, ReplayStatus::ReachedUntilNode);
    assert_eq!(result.events.len(), 2);
}

#[test]
fn replay_trace_jsonl_roundtrip_until_node() {
    let mut e1 = EngineEvent::new(EngineEventType::NodeReady);
    e1.node_id = Some("x1".to_string());
    let mut e2 = EngineEvent::new(EngineEventType::NodeReady);
    e2.node_id = Some("x2".to_string());
    let records = vec![
        EngineEventRecord::new("run-jsonl", 0, "2026-02-13T00:00:00Z", e1),
        EngineEventRecord::new("run-jsonl", 1, "2026-02-13T00:00:01Z", e2),
    ];
    let input = records
        .iter()
        .map(|record| encode_event_jsonl_line(record).expect("encode"))
        .collect::<String>();

    let result = replay_trace_jsonl(
        input.as_str(),
        &ReplayOptions {
            until_node: Some("x2".to_string()),
            max_steps: 8,
        },
    )
    .expect("replay");
    assert_eq!(result.status, ReplayStatus::ReachedUntilNode);
}

#[test]
fn replay_from_checkpoint_until_node_behavior() {
    let plan = sample_plan();
    let checkpoint = create_checkpoint_document(
        "run-cp",
        "plan-hash",
        CheckpointEngineState {
            completed_node_ids: vec!["n1".to_string()],
            paused_reason: Some("paused".to_string()),
            seen_command_ids: Vec::new(),
            pending_retries: Map::new(),
        },
        Some(json!({"inputs":{"amount":"1"}})),
        None,
    );
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = replay_from_checkpoint(
        &plan,
        &checkpoint,
        &router,
        &DefaultSolver,
        &crate::engine::EngineRunnerOptions::default(),
        &ReplayOptions {
            until_node: Some("n2".to_string()),
            max_steps: 4,
        },
    );

    assert_eq!(result.status, ReplayStatus::ReachedUntilNode);
    assert!(result.completed_node_ids.iter().any(|id| id == "n2"));
}

#[test]
fn replay_trace_jsonl_fixture_until_node() {
    let input = fs::read_to_string(fixture_root().join("trace/replay-trace.jsonl"))
        .expect("must read trace fixture");
    let result = replay_trace_jsonl(
        input.as_str(),
        &ReplayOptions {
            until_node: Some("n2".to_string()),
            max_steps: 8,
        },
    )
    .expect("replay fixture");
    assert_eq!(result.status, ReplayStatus::ReachedUntilNode);
    assert_eq!(result.events.len(), 2);
}

#[test]
fn replay_from_checkpoint_fixture_until_node() {
    let plan = load_plan_fixture("checkpoint/plan.json");
    let checkpoint = load_checkpoint_fixture("checkpoint/checkpoint.json");
    let mut router = RouterExecutor::new();
    router.register("evm", "eip155:1", Box::new(MockExecutor));

    let result = replay_from_checkpoint(
        &plan,
        &checkpoint,
        &router,
        &DefaultSolver,
        &crate::engine::EngineRunnerOptions::default(),
        &ReplayOptions {
            until_node: Some("n2".to_string()),
            max_steps: 4,
        },
    );

    assert_eq!(result.status, ReplayStatus::ReachedUntilNode);
    assert!(result.completed_node_ids.iter().any(|id| id == "n2"));
}

fn load_plan_fixture(relative: &str) -> PlanDocument {
    let path = fixture_root().join(relative);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read fixture {} failed: {error}", path.display()));
    serde_json::from_str::<PlanDocument>(content.as_str())
        .unwrap_or_else(|error| panic!("parse fixture {} failed: {error}", path.display()))
}

fn load_checkpoint_fixture(relative: &str) -> CheckpointDocument {
    let path = fixture_root().join(relative);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read fixture {} failed: {error}", path.display()));
    decode_checkpoint_json(content.as_str())
        .unwrap_or_else(|error| panic!("parse fixture {} failed: {error}", path.display()))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/plan-events")
}
