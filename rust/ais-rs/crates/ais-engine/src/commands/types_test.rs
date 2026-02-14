use super::{
    apply_command_with_dedupe, CommandDeduper, DuplicateCommandMode, EngineCommand,
    EngineCommandEnvelope, EngineCommandType, ENGINE_COMMAND_SCHEMA_0_0_1,
};
use crate::events::{EngineEventStream, EngineEventType};
use serde_json::json;

fn command_envelope(id: &str, command_type: EngineCommandType) -> EngineCommandEnvelope {
    EngineCommandEnvelope::new(EngineCommand {
        id: id.to_string(),
        command_type,
        data: serde_json::Map::from_iter([("scope".to_string(), json!("test"))]),
    })
}

#[test]
fn dedupe_accept_noop_mode_accepts_duplicate_with_noop() {
    let envelope = command_envelope("cmd-1", EngineCommandType::ApplyPatches);
    let mut deduper = CommandDeduper::new(DuplicateCommandMode::AcceptNoop);
    let mut stream = EngineEventStream::new("run-1");

    let first =
        apply_command_with_dedupe(&mut deduper, &mut stream, "2026-02-13T00:00:00Z", &envelope);
    assert!(first.accepted);
    assert!(!first.duplicate);
    assert_eq!(first.event_record.event.event_type, EngineEventType::CommandAccepted);

    let second =
        apply_command_with_dedupe(&mut deduper, &mut stream, "2026-02-13T00:00:01Z", &envelope);
    assert!(second.accepted);
    assert!(second.duplicate);
    assert_eq!(
        second.event_record.event.event_type,
        EngineEventType::CommandAccepted
    );
    assert_eq!(second.event_record.seq, first.event_record.seq + 1);
    assert_eq!(
        second.event_record.event.data.get("reason"),
        Some(&json!("duplicate_command_id"))
    );
}

#[test]
fn dedupe_reject_mode_rejects_duplicate_stably() {
    let envelope = command_envelope("cmd-2", EngineCommandType::Cancel);
    let mut deduper = CommandDeduper::new(DuplicateCommandMode::Reject);
    let mut stream = EngineEventStream::new("run-2");

    let first =
        apply_command_with_dedupe(&mut deduper, &mut stream, "2026-02-13T00:00:00Z", &envelope);
    assert!(first.accepted);
    assert!(!first.duplicate);

    let second =
        apply_command_with_dedupe(&mut deduper, &mut stream, "2026-02-13T00:00:01Z", &envelope);
    assert!(!second.accepted);
    assert!(second.duplicate);
    assert_eq!(
        second.event_record.event.event_type,
        EngineEventType::CommandRejected
    );
}

#[test]
fn with_seen_ids_includes_checkpoint_restored_ids() {
    let envelope = command_envelope("cmd-seen", EngineCommandType::UserConfirm);
    let mut deduper = CommandDeduper::with_seen_ids(
        DuplicateCommandMode::Reject,
        vec!["cmd-seen".to_string(), "cmd-other".to_string()],
    );
    let mut stream = EngineEventStream::new("run-3");

    let result =
        apply_command_with_dedupe(&mut deduper, &mut stream, "2026-02-13T00:00:00Z", &envelope);
    assert!(!result.accepted);
    assert!(result.duplicate);
    assert_eq!(
        deduper.seen_command_ids(),
        vec!["cmd-other".to_string(), "cmd-seen".to_string()]
    );
}

#[test]
fn envelope_schema_constant_is_applied() {
    let envelope = command_envelope("cmd-schema", EngineCommandType::SelectProvider);
    assert_eq!(envelope.schema, ENGINE_COMMAND_SCHEMA_0_0_1);
}
