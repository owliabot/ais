use crate::events::{EngineEvent, EngineEventRecord, EngineEventStream, EngineEventType};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const ENGINE_COMMAND_SCHEMA_0_0_1: &str = "ais-engine-command/0.0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCommandType {
    ApplyPatches,
    UserConfirm,
    SelectProvider,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineCommand {
    pub id: String,
    #[serde(rename = "type")]
    pub command_type: EngineCommandType,
    #[serde(default)]
    pub data: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineCommandEnvelope {
    pub schema: String,
    pub command: EngineCommand,
}

impl EngineCommandEnvelope {
    pub fn new(command: EngineCommand) -> Self {
        Self {
            schema: ENGINE_COMMAND_SCHEMA_0_0_1.to_string(),
            command,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateCommandMode {
    AcceptNoop,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandApplyResult {
    pub accepted: bool,
    pub duplicate: bool,
    pub event_record: EngineEventRecord,
}

#[derive(Debug, Clone)]
pub struct CommandDeduper {
    seen_command_ids: BTreeSet<String>,
    duplicate_mode: DuplicateCommandMode,
}

impl CommandDeduper {
    pub fn new(duplicate_mode: DuplicateCommandMode) -> Self {
        Self {
            seen_command_ids: BTreeSet::new(),
            duplicate_mode,
        }
    }

    pub fn with_seen_ids(
        duplicate_mode: DuplicateCommandMode,
        seen_command_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            seen_command_ids: seen_command_ids.into_iter().collect(),
            duplicate_mode,
        }
    }

    pub fn seen_command_ids(&self) -> Vec<String> {
        self.seen_command_ids.iter().cloned().collect()
    }
}

pub fn apply_command_with_dedupe(
    deduper: &mut CommandDeduper,
    stream: &mut EngineEventStream,
    ts: impl Into<String>,
    envelope: &EngineCommandEnvelope,
) -> CommandApplyResult {
    let command = &envelope.command;
    let is_duplicate = deduper.seen_command_ids.contains(&command.id);

    if is_duplicate {
        return match deduper.duplicate_mode {
            DuplicateCommandMode::AcceptNoop => CommandApplyResult {
                accepted: true,
                duplicate: true,
                event_record: stream.next_record(
                    ts,
                    command_event(
                        EngineEventType::CommandAccepted,
                        command,
                        true,
                        Some("duplicate_command_id"),
                    ),
                ),
            },
            DuplicateCommandMode::Reject => CommandApplyResult {
                accepted: false,
                duplicate: true,
                event_record: stream.next_record(
                    ts,
                    command_event(
                        EngineEventType::CommandRejected,
                        command,
                        true,
                        Some("duplicate_command_id"),
                    ),
                ),
            },
        };
    }

    deduper.seen_command_ids.insert(command.id.clone());
    CommandApplyResult {
        accepted: true,
        duplicate: false,
        event_record: stream.next_record(
            ts,
            command_event(EngineEventType::CommandAccepted, command, false, None),
        ),
    }
}

fn command_event(
    event_type: EngineEventType,
    command: &EngineCommand,
    duplicate: bool,
    reason: Option<&str>,
) -> EngineEvent {
    let mut event = EngineEvent::new(event_type);
    event.data.insert("command_id".to_string(), Value::String(command.id.clone()));
    event.data.insert(
        "command_type".to_string(),
        Value::String(command_type_name(command.command_type).to_string()),
    );
    event.data.insert("duplicate".to_string(), Value::Bool(duplicate));
    event.data.insert("noop".to_string(), Value::Bool(duplicate));
    if let Some(reason) = reason {
        event
            .data
            .insert("reason".to_string(), Value::String(reason.to_string()));
    }
    event
}

fn command_type_name(command_type: EngineCommandType) -> &'static str {
    match command_type {
        EngineCommandType::ApplyPatches => "apply_patches",
        EngineCommandType::UserConfirm => "user_confirm",
        EngineCommandType::SelectProvider => "select_provider",
        EngineCommandType::Cancel => "cancel",
    }
}

#[cfg(test)]
#[path = "types_test.rs"]
mod tests;
