use super::*;
use crate::audit_contract::AuditStreamAttempt;
use serde_json::Value;
use std::fs;

#[test]
fn compact_value_normalizes_whitespace() {
    assert_eq!(compact_value(" a \n  b\t c "), "a b c");
}

#[test]
fn persisted_agent_trace_sink_writes_host_decision_jsonl() {
    let path = std::env::temp_dir().join(format!(
        "ais-runner-agent-trace-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let guard = install_jsonl_sink(
        Some(path.as_path()),
        "run-test",
        &AuditStreamAttempt::for_index(3),
    )
    .expect("install trace sink");
    emit(
        false,
        "grounding",
        "ready",
        &[("missing_refs", "inputs.token.decimals".to_string())],
    );
    drop(guard);

    let text = fs::read_to_string(path.as_path()).expect("read trace file");
    let line = text.lines().next().expect("trace line");
    let value: Value = serde_json::from_str(line).expect("json");
    assert_eq!(
        value.get("schema"),
        Some(&Value::String("ais-runner-agent-trace/0.0.1".to_string()))
    );
    assert_eq!(
        value.get("run_id"),
        Some(&Value::String("run-test".to_string()))
    );
    assert_eq!(
        value.get("attempt_id"),
        Some(&Value::String("attempt-3".to_string()))
    );
    assert_eq!(value.get("attempt_index"), Some(&Value::from(3)));
    assert_eq!(
        value.get("seq_scope"),
        Some(&Value::String("attempt_local".to_string()))
    );
    assert_eq!(value.get("seq"), Some(&Value::from(0)));
    assert_eq!(
        value.get("phase"),
        Some(&Value::String("grounding".to_string()))
    );
    assert_eq!(
        value.get("event"),
        Some(&Value::String("ready".to_string()))
    );
    assert_eq!(
        value.pointer("/fields/missing_refs"),
        Some(&Value::String("inputs.token.decimals".to_string()))
    );

    let _ = fs::remove_file(path);
}

#[test]
fn flush_pending_writes_resume_records_after_sink_install() {
    let path = std::env::temp_dir().join(format!(
        "ais-runner-agent-trace-pending-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let guard = install_jsonl_sink(
        Some(path.as_path()),
        "run-resume",
        &AuditStreamAttempt::for_index(2),
    )
    .expect("install trace sink");
    flush_pending(
        false,
        &[PendingTraceRecord::new(
            "resume",
            "resume_skip_confirmed_write",
            vec![
                ("count".to_string(), "1".to_string()),
                ("node_ids".to_string(), "swap-1".to_string()),
            ],
        )],
    );
    ensure_sink_healthy().expect("pending trace flush");
    drop(guard);

    let text = fs::read_to_string(path.as_path()).expect("read trace file");
    let line = text.lines().next().expect("trace line");
    let value: Value = serde_json::from_str(line).expect("json");
    assert_eq!(
        value.get("phase"),
        Some(&Value::String("resume".to_string()))
    );
    assert_eq!(
        value.get("event"),
        Some(&Value::String("resume_skip_confirmed_write".to_string()))
    );
    assert_eq!(
        value.pointer("/fields/count"),
        Some(&Value::String("1".to_string()))
    );

    let _ = fs::remove_file(path);
}
