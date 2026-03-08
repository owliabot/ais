use ais_engine::EngineEventRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) const AUDIT_EXTENSION_KEY: &str = "audit_stream";
const KEY_RESUME_CORE: &str = "resume_core";
const AUDIT_SCHEMA: &str = "ais-runner-audit-stream/0.0.1";
const SEQ_SCOPE_ATTEMPT_LOCAL: &str = "attempt_local";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AuditStreamAttempt {
    pub(crate) schema: String,
    pub(crate) attempt_index: u64,
    pub(crate) attempt_id: String,
    pub(crate) seq_scope: String,
    #[serde(default)]
    pub(crate) last_event_attempt_id: Option<String>,
    #[serde(default)]
    pub(crate) last_event_attempt_index: Option<u64>,
    #[serde(default)]
    pub(crate) last_event_seq: Option<u64>,
    #[serde(default)]
    pub(crate) last_event_ts: Option<String>,
    #[serde(default)]
    pub(crate) last_event_run_id: Option<String>,
}

impl AuditStreamAttempt {
    pub(crate) fn fresh() -> Self {
        Self::for_index(1)
    }

    pub(crate) fn for_index(attempt_index: u64) -> Self {
        let index = attempt_index.max(1);
        Self {
            schema: AUDIT_SCHEMA.to_string(),
            attempt_index: index,
            attempt_id: format!("attempt-{index}"),
            seq_scope: SEQ_SCOPE_ATTEMPT_LOCAL.to_string(),
            last_event_attempt_id: None,
            last_event_attempt_index: None,
            last_event_seq: None,
            last_event_ts: None,
            last_event_run_id: None,
        }
    }

    pub(crate) fn record_persisted_events(&mut self, events: &[EngineEventRecord]) {
        let Some(last) = events.last() else {
            return;
        };
        self.last_event_attempt_id = Some(self.attempt_id.clone());
        self.last_event_attempt_index = Some(self.attempt_index);
        self.last_event_seq = Some(last.seq);
        self.last_event_ts = Some(last.ts.clone());
        self.last_event_run_id = Some(last.run_id.clone());
    }

    pub(crate) fn record_persisted_events_if(
        &mut self,
        persisted: bool,
        events: &[EngineEventRecord],
    ) {
        if persisted {
            self.record_persisted_events(events);
        }
    }
}

pub(crate) fn current_attempt_from_extensions(
    extensions: Option<&Map<String, Value>>,
) -> Option<AuditStreamAttempt> {
    let raw = extensions
        .and_then(|root| {
            root.get(KEY_RESUME_CORE)
                .and_then(Value::as_object)
                .and_then(|section| section.get(AUDIT_EXTENSION_KEY))
        })?
        .clone();
    serde_json::from_value(raw).ok()
}

pub(crate) fn next_attempt_from_extensions(
    extensions: Option<&Map<String, Value>>,
) -> AuditStreamAttempt {
    let previous = current_attempt_from_extensions(extensions);
    let next_index = previous
        .as_ref()
        .map(|attempt| attempt.attempt_index.saturating_add(1))
        .unwrap_or(1);
    let mut next = AuditStreamAttempt::for_index(next_index);
    if let Some(previous) = previous {
        next.last_event_attempt_id = previous.last_event_attempt_id;
        next.last_event_attempt_index = previous.last_event_attempt_index;
        next.last_event_seq = previous.last_event_seq;
        next.last_event_ts = previous.last_event_ts;
        next.last_event_run_id = previous.last_event_run_id;
    }
    next
}

pub(crate) fn write_attempt_into_extensions(
    extensions: &mut Map<String, Value>,
    attempt: &AuditStreamAttempt,
) {
    if let Ok(value) = serde_json::to_value(attempt) {
        let resume_core = extensions
            .entry(KEY_RESUME_CORE.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !resume_core.is_object() {
            *resume_core = Value::Object(Map::new());
        }
        if let Some(object) = resume_core.as_object_mut() {
            object.insert(AUDIT_EXTENSION_KEY.to_string(), value);
        }
    }
}

pub(crate) fn augment_jsonl_line(
    line: &str,
    attempt: &AuditStreamAttempt,
) -> Result<String, serde_json::Error> {
    let mut value = serde_json::from_str::<Value>(line)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "attempt_id".to_string(),
            Value::String(attempt.attempt_id.clone()),
        );
        object.insert(
            "attempt_index".to_string(),
            Value::Number(attempt.attempt_index.into()),
        );
        object.insert(
            "seq_scope".to_string(),
            Value::String(attempt.seq_scope.clone()),
        );
    }
    let mut encoded = serde_json::to_string(&value)?;
    encoded.push('\n');
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn next_attempt_from_extensions_increments_attempt_identity() {
        let mut extensions = Map::new();
        let mut current = AuditStreamAttempt::for_index(2);
        current.last_event_seq = Some(7);
        current.last_event_ts = Some("2026-03-06T00:00:00Z".to_string());
        write_attempt_into_extensions(&mut extensions, &current);
        let next = next_attempt_from_extensions(Some(&extensions));
        assert_eq!(next.attempt_index, 3);
        assert_eq!(next.attempt_id, "attempt-3");
        assert_eq!(next.seq_scope, "attempt_local");
        assert_eq!(next.last_event_seq, Some(7));
        assert_eq!(next.last_event_ts.as_deref(), Some("2026-03-06T00:00:00Z"));
    }

    #[test]
    fn augment_jsonl_line_adds_attempt_metadata() {
        let line = json!({
            "schema": "ais-engine-event/0.0.3",
            "run_id": "run-123",
            "seq": 0
        })
        .to_string();
        let encoded =
            augment_jsonl_line(&line, &AuditStreamAttempt::for_index(4)).expect("augment");
        let value: Value = serde_json::from_str(encoded.trim()).expect("json");
        assert_eq!(value.get("attempt_id"), Some(&json!("attempt-4")));
        assert_eq!(value.get("attempt_index"), Some(&json!(4)));
        assert_eq!(value.get("seq_scope"), Some(&json!("attempt_local")));
    }

    #[test]
    fn record_persisted_events_updates_watermark() {
        let mut attempt = AuditStreamAttempt::for_index(2);
        let events = vec![EngineEventRecord {
            schema: "ais-engine-event/0.0.3".to_string(),
            run_id: "run-abc".to_string(),
            seq: 5,
            ts: "2026-03-06T01:02:03Z".to_string(),
            event: ais_engine::EngineEvent::new(ais_engine::EngineEventType::EnginePaused),
        }];
        attempt.record_persisted_events(&events);
        assert_eq!(attempt.last_event_attempt_id.as_deref(), Some("attempt-2"));
        assert_eq!(attempt.last_event_attempt_index, Some(2));
        assert_eq!(attempt.last_event_seq, Some(5));
        assert_eq!(
            attempt.last_event_ts.as_deref(),
            Some("2026-03-06T01:02:03Z")
        );
        assert_eq!(attempt.last_event_run_id.as_deref(), Some("run-abc"));
    }
}
