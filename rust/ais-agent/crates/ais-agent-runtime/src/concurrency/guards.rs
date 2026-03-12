use serde::{Deserialize, Serialize};

use ais_agent_control::{
    commands::{ExpectedRuntimeVersion, RunCommand},
    ids::RunId,
};

use crate::{concurrency::RuntimeVersion, runtime::ActiveRun};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandVersionMismatchField {
    CheckpointSeq,
    PlanEpoch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandVersionMismatch {
    pub field: CommandVersionMismatchField,
    pub expected: u64,
    pub actual: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandVersionConflict {
    pub code: String,
    pub run_id: RunId,
    pub command_kind: String,
    pub current: RuntimeVersion,
    pub mismatches: Vec<CommandVersionMismatch>,
}

pub fn guard_run_command_version(
    command: &RunCommand,
    runtime: &ActiveRun,
) -> Result<RuntimeVersion, CommandVersionConflict> {
    let current = RuntimeVersion::from_runtime(runtime);
    let Some(expected) = command.expected_runtime_version() else {
        return Ok(current);
    };

    if expected.is_empty() {
        return Ok(current);
    }

    let mismatches = collect_mismatches(expected, current);
    if mismatches.is_empty() {
        return Ok(current);
    }

    Err(CommandVersionConflict {
        code: "stale_command_conflict".to_string(),
        run_id: runtime.run_id.clone(),
        command_kind: command.kind().to_string(),
        current,
        mismatches,
    })
}

fn collect_mismatches(
    expected: &ExpectedRuntimeVersion,
    current: RuntimeVersion,
) -> Vec<CommandVersionMismatch> {
    let mut mismatches = Vec::new();

    if let Some(expected_checkpoint_seq) = expected.checkpoint_seq {
        if expected_checkpoint_seq != current.checkpoint_seq {
            mismatches.push(CommandVersionMismatch {
                field: CommandVersionMismatchField::CheckpointSeq,
                expected: expected_checkpoint_seq,
                actual: current.checkpoint_seq,
            });
        }
    }

    if let Some(expected_plan_epoch) = expected.plan_epoch {
        if expected_plan_epoch != current.plan_epoch {
            mismatches.push(CommandVersionMismatch {
                field: CommandVersionMismatchField::PlanEpoch,
                expected: expected_plan_epoch,
                actual: current.plan_epoch,
            });
        }
    }

    mismatches
}
