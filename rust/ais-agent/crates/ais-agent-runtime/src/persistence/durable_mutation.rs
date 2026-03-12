use ais_agent_control::{audit::RuntimeAuditRecord, events::RunEventEnvelope, ids::RunId};
use ais_agent_core::{mission::Mission, runtime::SignerRequestState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::persistence::{CheckpointArchiveEntry, RunCatalogEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableMutationKind {
    RunBegin,
    Progress,
    SideEffect,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionWriteMode {
    Insert,
    Upsert,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionWrite {
    pub run_id: RunId,
    pub mode: MissionWriteMode,
    pub mission: Mission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointWrite {
    pub entry: CheckpointArchiveEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWriteBatch {
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogWrite {
    pub entry: RunCatalogEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignerStateWrite {
    Upsert { signer_state: SignerRequestState },
    Clear { run_id: RunId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditWriteBatch {
    #[serde(default)]
    pub records: Vec<RuntimeAuditRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableMutationUnit {
    pub run_id: RunId,
    pub kind: DurableMutationKind,
    pub mission_write: Option<MissionWrite>,
    pub checkpoint_write: CheckpointWrite,
    pub event_write: EventWriteBatch,
    pub catalog_write: CatalogWrite,
    pub signer_write: Option<SignerStateWrite>,
    pub audit_write: AuditWriteBatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DurableMutationContractError {
    #[error("run_begin unit for `{run_id}` requires mission insert")]
    RunBeginRequiresMissionInsert { run_id: String },
    #[error("mission write run `{mission_run_id}` does not match unit run `{unit_run_id}`")]
    MissionRunIdMismatch {
        unit_run_id: String,
        mission_run_id: String,
    },
    #[error("checkpoint write run `{checkpoint_run_id}` does not match unit run `{unit_run_id}`")]
    CheckpointRunIdMismatch {
        unit_run_id: String,
        checkpoint_run_id: String,
    },
    #[error("catalog write run `{catalog_run_id}` does not match unit run `{unit_run_id}`")]
    CatalogRunIdMismatch {
        unit_run_id: String,
        catalog_run_id: String,
    },
    #[error("catalog latest checkpoint `{catalog_checkpoint_seq}` does not match checkpoint write `{checkpoint_seq}` for run `{run_id}`")]
    CatalogCheckpointMismatch {
        run_id: String,
        catalog_checkpoint_seq: u64,
        checkpoint_seq: u64,
    },
    #[error("catalog latest event `{catalog_latest_event_seq:?}` does not match event batch tail `{event_batch_tail:?}` for run `{run_id}`")]
    CatalogLatestEventMismatch {
        run_id: String,
        catalog_latest_event_seq: Option<u64>,
        event_batch_tail: Option<u64>,
    },
    #[error("event batch for run `{run_id}` contains run `{event_run_id}` at seq `{event_seq}`")]
    EventRunIdMismatch {
        run_id: String,
        event_run_id: String,
        event_seq: u64,
    },
    #[error("event batch is not strictly monotonic for run `{run_id}`")]
    EventBatchNotMonotonic { run_id: String },
    #[error("signer write run `{signer_run_id}` does not match unit run `{unit_run_id}`")]
    SignerRunIdMismatch {
        unit_run_id: String,
        signer_run_id: String,
    },
    #[error("audit batch for run `{run_id}` contains run `{audit_run_id}` at seq `{audit_seq}`")]
    AuditRunIdMismatch {
        run_id: String,
        audit_run_id: String,
        audit_seq: u64,
    },
    #[error("audit batch is not strictly monotonic for run `{run_id}`")]
    AuditBatchNotMonotonic { run_id: String },
}

pub fn validate_durable_mutation_unit(
    unit: &DurableMutationUnit,
) -> Result<(), DurableMutationContractError> {
    if matches!(unit.kind, DurableMutationKind::RunBegin)
        && !matches!(
            unit.mission_write.as_ref().map(|write| write.mode),
            Some(MissionWriteMode::Insert)
        )
    {
        return Err(
            DurableMutationContractError::RunBeginRequiresMissionInsert {
                run_id: unit.run_id.0.clone(),
            },
        );
    }

    if let Some(write) = unit.mission_write.as_ref() {
        if write.run_id != unit.run_id {
            return Err(DurableMutationContractError::MissionRunIdMismatch {
                unit_run_id: unit.run_id.0.clone(),
                mission_run_id: write.run_id.0.clone(),
            });
        }
    }

    if unit.checkpoint_write.entry.snapshot.run_id != unit.run_id.0 {
        return Err(DurableMutationContractError::CheckpointRunIdMismatch {
            unit_run_id: unit.run_id.0.clone(),
            checkpoint_run_id: unit.checkpoint_write.entry.snapshot.run_id.clone(),
        });
    }

    if unit.catalog_write.entry.run_id != unit.run_id {
        return Err(DurableMutationContractError::CatalogRunIdMismatch {
            unit_run_id: unit.run_id.0.clone(),
            catalog_run_id: unit.catalog_write.entry.run_id.0.clone(),
        });
    }

    if unit.catalog_write.entry.latest_checkpoint_seq
        != unit.checkpoint_write.entry.snapshot.checkpoint_seq
    {
        return Err(DurableMutationContractError::CatalogCheckpointMismatch {
            run_id: unit.run_id.0.clone(),
            catalog_checkpoint_seq: unit.catalog_write.entry.latest_checkpoint_seq,
            checkpoint_seq: unit.checkpoint_write.entry.snapshot.checkpoint_seq,
        });
    }

    let mut last_event_seq = None;
    for event in &unit.event_write.events {
        if event.run_id != unit.run_id {
            return Err(DurableMutationContractError::EventRunIdMismatch {
                run_id: unit.run_id.0.clone(),
                event_run_id: event.run_id.0.clone(),
                event_seq: event.event_seq,
            });
        }
        if last_event_seq.is_some_and(|previous| event.event_seq <= previous) {
            return Err(DurableMutationContractError::EventBatchNotMonotonic {
                run_id: unit.run_id.0.clone(),
            });
        }
        last_event_seq = Some(event.event_seq);
    }

    if last_event_seq.is_some() && unit.catalog_write.entry.latest_event_seq != last_event_seq {
        return Err(DurableMutationContractError::CatalogLatestEventMismatch {
            run_id: unit.run_id.0.clone(),
            catalog_latest_event_seq: unit.catalog_write.entry.latest_event_seq,
            event_batch_tail: last_event_seq,
        });
    }

    if let Some(write) = unit.signer_write.as_ref() {
        let signer_run_id = match write {
            SignerStateWrite::Upsert { signer_state } => &signer_state.run_id,
            SignerStateWrite::Clear { run_id } => run_id,
        };
        if signer_run_id != &unit.run_id {
            return Err(DurableMutationContractError::SignerRunIdMismatch {
                unit_run_id: unit.run_id.0.clone(),
                signer_run_id: signer_run_id.0.clone(),
            });
        }
    }

    let mut last_audit_seq = None;
    for record in &unit.audit_write.records {
        if record.run_id != unit.run_id {
            return Err(DurableMutationContractError::AuditRunIdMismatch {
                run_id: unit.run_id.0.clone(),
                audit_run_id: record.run_id.0.clone(),
                audit_seq: record.audit_seq,
            });
        }
        if last_audit_seq.is_some_and(|previous| record.audit_seq <= previous) {
            return Err(DurableMutationContractError::AuditBatchNotMonotonic {
                run_id: unit.run_id.0.clone(),
            });
        }
        last_audit_seq = Some(record.audit_seq);
    }

    Ok(())
}
