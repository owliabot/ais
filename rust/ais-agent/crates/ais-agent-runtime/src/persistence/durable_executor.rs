use ais_agent_control::ids::RunId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::persistence::{
    validate_durable_mutation_unit, CheckpointRepository, DurableMutationContractError,
    DurableMutationKind, DurableMutationUnit, EventArchive, MissionRepository, MissionWriteMode,
    RunCatalogRepository, RuntimeAuditArchive, SignerStateArchive,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableMutationMember {
    Mission,
    Checkpoint,
    Event,
    Catalog,
    Signer,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableCommitReceipt {
    pub run_id: RunId,
    pub kind: DurableMutationKind,
    pub checkpoint_seq: u64,
    pub plan_epoch: u64,
    pub latest_event_seq: Option<u64>,
    pub latest_audit_seq: Option<u64>,
}

#[derive(Debug, Error)]
pub enum DurableCommitError {
    #[error("durable mutation contract invalid: {0}")]
    InvalidUnit(#[from] DurableMutationContractError),
    #[error("durable mutation transaction `{phase}` failed for run `{run_id}`: {message}")]
    Transaction {
        run_id: String,
        phase: String,
        message: String,
    },
    #[error("durable mutation member `{member:?}` failed for run `{run_id}`: {message}")]
    MemberWrite {
        run_id: String,
        member: DurableMutationMember,
        message: String,
    },
}

pub trait DurableMutationExecutor {
    fn commit(
        &mut self,
        unit: DurableMutationUnit,
    ) -> Result<DurableCommitReceipt, DurableCommitError>;
}

#[derive(Debug)]
pub struct LinearDurableMutationExecutor<M, C, E, K, G, A> {
    mission_repo: M,
    checkpoint_repo: C,
    event_archive: E,
    run_catalog_repo: K,
    signer_archive: G,
    audit_archive: A,
}

impl<M, C, E, K, G, A> LinearDurableMutationExecutor<M, C, E, K, G, A> {
    pub fn new(
        mission_repo: M,
        checkpoint_repo: C,
        event_archive: E,
        run_catalog_repo: K,
        signer_archive: G,
        audit_archive: A,
    ) -> Self {
        Self {
            mission_repo,
            checkpoint_repo,
            event_archive,
            run_catalog_repo,
            signer_archive,
            audit_archive,
        }
    }

    pub fn into_parts(self) -> (M, C, E, K, G, A) {
        (
            self.mission_repo,
            self.checkpoint_repo,
            self.event_archive,
            self.run_catalog_repo,
            self.signer_archive,
            self.audit_archive,
        )
    }
}

impl<M, C, E, K, G, A> DurableMutationExecutor for LinearDurableMutationExecutor<M, C, E, K, G, A>
where
    M: MissionRepository,
    C: CheckpointRepository,
    E: EventArchive,
    K: RunCatalogRepository,
    G: SignerStateArchive,
    A: RuntimeAuditArchive,
{
    fn commit(
        &mut self,
        unit: DurableMutationUnit,
    ) -> Result<DurableCommitReceipt, DurableCommitError> {
        validate_durable_mutation_unit(&unit)?;

        if let Some(write) = unit.mission_write.as_ref() {
            let result = match write.mode {
                MissionWriteMode::Insert => self
                    .mission_repo
                    .insert(write.run_id.clone(), write.mission.clone()),
                MissionWriteMode::Upsert => self
                    .mission_repo
                    .upsert(write.run_id.clone(), write.mission.clone()),
            };
            result.map_err(|error| DurableCommitError::MemberWrite {
                run_id: unit.run_id.0.clone(),
                member: DurableMutationMember::Mission,
                message: error.to_string(),
            })?;
        }

        self.checkpoint_repo
            .append(unit.checkpoint_write.entry.clone())
            .map_err(|error| DurableCommitError::MemberWrite {
                run_id: unit.run_id.0.clone(),
                member: DurableMutationMember::Checkpoint,
                message: error.to_string(),
            })?;

        for event in &unit.event_write.events {
            self.event_archive.append(event.clone()).map_err(|error| {
                DurableCommitError::MemberWrite {
                    run_id: unit.run_id.0.clone(),
                    member: DurableMutationMember::Event,
                    message: error.to_string(),
                }
            })?;
        }

        self.run_catalog_repo
            .upsert(unit.catalog_write.entry.clone())
            .map_err(|error| DurableCommitError::MemberWrite {
                run_id: unit.run_id.0.clone(),
                member: DurableMutationMember::Catalog,
                message: error.to_string(),
            })?;

        if let Some(write) = unit.signer_write.as_ref() {
            let result = match write {
                crate::persistence::SignerStateWrite::Upsert { signer_state } => {
                    self.signer_archive.upsert(signer_state.clone())
                }
                crate::persistence::SignerStateWrite::Clear { run_id } => {
                    self.signer_archive.clear(run_id)
                }
            };
            result.map_err(|error| DurableCommitError::MemberWrite {
                run_id: unit.run_id.0.clone(),
                member: DurableMutationMember::Signer,
                message: error.to_string(),
            })?;
        }

        for record in &unit.audit_write.records {
            self.audit_archive.append(record.clone()).map_err(|error| {
                DurableCommitError::MemberWrite {
                    run_id: unit.run_id.0.clone(),
                    member: DurableMutationMember::Audit,
                    message: error.to_string(),
                }
            })?;
        }

        Ok(DurableCommitReceipt {
            run_id: unit.run_id,
            kind: unit.kind,
            checkpoint_seq: unit.checkpoint_write.entry.snapshot.checkpoint_seq,
            plan_epoch: unit.checkpoint_write.entry.snapshot.plan_epoch,
            latest_event_seq: unit.event_write.events.last().map(|event| event.event_seq),
            latest_audit_seq: unit
                .audit_write
                .records
                .last()
                .map(|record| record.audit_seq),
        })
    }
}
