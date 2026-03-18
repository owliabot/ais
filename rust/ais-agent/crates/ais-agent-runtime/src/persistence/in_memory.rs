//! In-memory repository shells for hot tests and contract freeze coverage.

use std::collections::BTreeMap;

use crate::persistence::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, ClaimExpireRequest,
    ClaimReleaseRequest, ClaimRenewRequest, ClaimSupersedeRequest, ClaimSupersedeResult,
    EventArchive, EventArchiveError, EventArchiveQuery, EventArchiveSlice, MissionRepository,
    MissionRepositoryError, RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError,
    RunClaimRepository, RunClaimRepositoryError, RunWaitStateRecord, RunWaitStateStore,
    RunWaitStateStoreError, RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery,
    RuntimeAuditSlice,
};
use ais_agent_control::{
    audit::RuntimeAuditRecord,
    events::RunEventEnvelope,
    ids::{ClaimId, RunId},
    ownership::{RunClaim, RunClaimStatus},
};
use ais_agent_core::checkpoint::CheckpointSnapshot;
use ais_agent_core::mission::Mission;

#[derive(Debug, Default)]
pub struct InMemoryCheckpointRepository {
    snapshots: BTreeMap<String, Vec<CheckpointArchiveEntry>>,
}

impl InMemoryCheckpointRepository {
    pub fn history_len(&self, run_id: &str) -> usize {
        self.snapshots
            .get(run_id)
            .map(|history| history.len())
            .unwrap_or_default()
    }
}

impl CheckpointArchive for InMemoryCheckpointRepository {
    fn latest(&self, run_id: &str) -> Result<CheckpointSnapshot, CheckpointArchiveError> {
        self.snapshots
            .get(run_id)
            .and_then(|history| {
                history
                    .iter()
                    .max_by(|left, right| {
                        left.snapshot
                            .checkpoint_seq
                            .cmp(&right.snapshot.checkpoint_seq)
                            .then(left.snapshot.plan_epoch.cmp(&right.snapshot.plan_epoch))
                    })
                    .cloned()
            })
            .map(|entry| entry.snapshot)
            .ok_or_else(|| CheckpointArchiveError::NotFound {
                run_id: run_id.to_owned(),
            })
    }

    fn append(&mut self, entry: CheckpointArchiveEntry) -> Result<(), CheckpointArchiveError> {
        self.snapshots
            .entry(entry.snapshot.run_id.clone())
            .or_default()
            .push(entry);
        Ok(())
    }

    fn history(&self, run_id: &str) -> Result<Vec<CheckpointArchiveEntry>, CheckpointArchiveError> {
        self.snapshots
            .get(run_id)
            .cloned()
            .ok_or_else(|| CheckpointArchiveError::NotFound {
                run_id: run_id.to_owned(),
            })
    }
}

#[derive(Debug, Default)]
pub struct InMemoryMissionRepository {
    missions: BTreeMap<String, Mission>,
}

impl MissionRepository for InMemoryMissionRepository {
    fn insert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        if self.missions.contains_key(&run_id.0) {
            return Err(MissionRepositoryError::AlreadyExists { run_id: run_id.0 });
        }

        self.missions.insert(run_id.0, mission);
        Ok(())
    }

    fn upsert(&mut self, run_id: RunId, mission: Mission) -> Result<(), MissionRepositoryError> {
        self.missions.insert(run_id.0, mission);
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<Mission, MissionRepositoryError> {
        self.missions
            .get(&run_id.0)
            .cloned()
            .ok_or_else(|| MissionRepositoryError::NotFound {
                run_id: run_id.0.clone(),
            })
    }
}

#[derive(Debug, Default)]
pub struct InMemoryRunCatalogRepository {
    entries: BTreeMap<String, RunCatalogEntry>,
}

#[derive(Debug, Default)]
pub struct InMemoryRunClaimRepository {
    claims_by_id: BTreeMap<String, RunClaim>,
    active_claim_by_run: BTreeMap<String, String>,
}

impl InMemoryRunClaimRepository {
    fn validate_active_claim(claim: &RunClaim) -> Result<(), RunClaimRepositoryError> {
        claim
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;
        if claim.status != RunClaimStatus::Active {
            return Err(RunClaimRepositoryError::InvalidStatus {
                claim_id: claim.claim_id.0.clone(),
                status: claim.status.clone(),
            });
        }
        Ok(())
    }

    fn load_existing_claim(&self, claim_id: &ClaimId) -> Result<RunClaim, RunClaimRepositoryError> {
        self.claims_by_id.get(&claim_id.0).cloned().ok_or_else(|| {
            RunClaimRepositoryError::ClaimNotFound {
                claim_id: claim_id.0.clone(),
            }
        })
    }
}

impl RunClaimRepository for InMemoryRunClaimRepository {
    fn acquire(&mut self, claim: RunClaim) -> Result<RunClaim, RunClaimRepositoryError> {
        Self::validate_active_claim(&claim)?;
        if let Some(active_claim_id) = self.active_claim_by_run.get(&claim.run_id.0) {
            return Err(RunClaimRepositoryError::ActiveClaimConflict {
                run_id: claim.run_id.0.clone(),
                claim_id: active_claim_id.clone(),
            });
        }

        self.active_claim_by_run
            .insert(claim.run_id.0.clone(), claim.claim_id.0.clone());
        self.claims_by_id
            .insert(claim.claim_id.0.clone(), claim.clone());
        Ok(claim)
    }

    fn renew(&mut self, request: ClaimRenewRequest) -> Result<RunClaim, RunClaimRepositoryError> {
        let mut current = self.load_existing_claim(&request.claim_id)?;
        if current.run_id != request.run_id {
            return Err(RunClaimRepositoryError::InvalidClaim {
                message: "claim renew run_id does not match existing claim".to_owned(),
            });
        }
        if current.status != RunClaimStatus::Active {
            return Err(RunClaimRepositoryError::InvalidStatus {
                claim_id: current.claim_id.0.clone(),
                status: current.status,
            });
        }
        if current.claim_epoch != request.claim_epoch {
            return Err(RunClaimRepositoryError::ClaimEpochConflict {
                claim_id: current.claim_id.0.clone(),
                expected_claim_epoch: request.claim_epoch,
                actual_claim_epoch: current.claim_epoch,
            });
        }

        current.last_renewed_at_ms = Some(request.renewed_at_ms);
        current.lease_expires_at_ms = request.lease_expires_at_ms;
        current.claim_epoch += 1;
        current
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;
        self.claims_by_id
            .insert(current.claim_id.0.clone(), current.clone());
        Ok(current)
    }

    fn release(
        &mut self,
        request: ClaimReleaseRequest,
    ) -> Result<RunClaim, RunClaimRepositoryError> {
        let mut current = self.load_existing_claim(&request.claim_id)?;
        if current.run_id != request.run_id {
            return Err(RunClaimRepositoryError::InvalidClaim {
                message: "claim release run_id does not match existing claim".to_owned(),
            });
        }
        if current.status != RunClaimStatus::Active {
            return Err(RunClaimRepositoryError::InvalidStatus {
                claim_id: current.claim_id.0.clone(),
                status: current.status,
            });
        }
        if current.claim_epoch != request.claim_epoch {
            return Err(RunClaimRepositoryError::ClaimEpochConflict {
                claim_id: current.claim_id.0.clone(),
                expected_claim_epoch: request.claim_epoch,
                actual_claim_epoch: current.claim_epoch,
            });
        }

        current.status = RunClaimStatus::Released;
        current.claim_epoch += 1;
        current
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;
        self.claims_by_id
            .insert(current.claim_id.0.clone(), current.clone());
        self.active_claim_by_run.remove(&current.run_id.0);
        Ok(current)
    }

    fn load_active(&self, run_id: &RunId) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        Ok(self
            .active_claim_by_run
            .get(&run_id.0)
            .and_then(|claim_id| self.claims_by_id.get(claim_id))
            .cloned())
    }

    fn load_latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        Ok(self
            .claims_by_id
            .values()
            .filter(|claim| claim.run_id == *run_id)
            .cloned()
            .max_by(|left, right| {
                (
                    left.lease_started_at_ms,
                    left.last_renewed_at_ms.unwrap_or(left.lease_started_at_ms),
                    left.claim_epoch,
                    &left.claim_id.0,
                )
                    .cmp(&(
                        right.lease_started_at_ms,
                        right
                            .last_renewed_at_ms
                            .unwrap_or(right.lease_started_at_ms),
                        right.claim_epoch,
                        &right.claim_id.0,
                    ))
            }))
    }

    fn load_claim(&self, claim_id: &ClaimId) -> Result<RunClaim, RunClaimRepositoryError> {
        self.load_existing_claim(claim_id)
    }

    fn expire_stale(
        &mut self,
        request: ClaimExpireRequest,
    ) -> Result<Option<RunClaim>, RunClaimRepositoryError> {
        let Some(claim_id) = self.active_claim_by_run.get(&request.run_id.0).cloned() else {
            return Ok(None);
        };
        let mut current = self.load_existing_claim(&ClaimId(claim_id))?;
        let Some(lease_expires_at_ms) = current.lease_expires_at_ms else {
            return Ok(None);
        };
        if lease_expires_at_ms > request.now_ms {
            return Ok(None);
        }
        if current.status != RunClaimStatus::Active {
            return Ok(None);
        }

        current.status = RunClaimStatus::Expired;
        current.claim_epoch += 1;
        current
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;
        self.claims_by_id
            .insert(current.claim_id.0.clone(), current.clone());
        self.active_claim_by_run.remove(&current.run_id.0);
        Ok(Some(current))
    }

    fn supersede(
        &mut self,
        request: ClaimSupersedeRequest,
    ) -> Result<ClaimSupersedeResult, RunClaimRepositoryError> {
        Self::validate_active_claim(&request.successor_claim)?;
        if request.successor_claim.run_id != request.run_id {
            return Err(RunClaimRepositoryError::InvalidClaim {
                message: "successor claim run_id does not match supersede request".to_owned(),
            });
        }

        let active_claim_id = self
            .active_claim_by_run
            .get(&request.run_id.0)
            .cloned()
            .ok_or_else(|| RunClaimRepositoryError::ClaimNotFound {
                claim_id: request.predecessor_claim_id.0.clone(),
            })?;
        if active_claim_id != request.predecessor_claim_id.0 {
            return Err(RunClaimRepositoryError::ActiveClaimConflict {
                run_id: request.run_id.0.clone(),
                claim_id: active_claim_id,
            });
        }

        let mut predecessor = self.load_existing_claim(&request.predecessor_claim_id)?;
        if predecessor.claim_epoch != request.predecessor_claim_epoch {
            return Err(RunClaimRepositoryError::ClaimEpochConflict {
                claim_id: predecessor.claim_id.0.clone(),
                expected_claim_epoch: request.predecessor_claim_epoch,
                actual_claim_epoch: predecessor.claim_epoch,
            });
        }
        if predecessor.status != RunClaimStatus::Active {
            return Err(RunClaimRepositoryError::InvalidStatus {
                claim_id: predecessor.claim_id.0.clone(),
                status: predecessor.status,
            });
        }

        predecessor.status = RunClaimStatus::Superseded;
        predecessor.claim_epoch += 1;
        predecessor
            .validate()
            .map_err(|message| RunClaimRepositoryError::InvalidClaim { message })?;

        self.claims_by_id
            .insert(predecessor.claim_id.0.clone(), predecessor.clone());
        self.claims_by_id.insert(
            request.successor_claim.claim_id.0.clone(),
            request.successor_claim.clone(),
        );
        self.active_claim_by_run.insert(
            request.run_id.0.clone(),
            request.successor_claim.claim_id.0.clone(),
        );

        Ok(ClaimSupersedeResult {
            predecessor,
            successor: request.successor_claim,
        })
    }
}

impl RunCatalogRepository for InMemoryRunCatalogRepository {
    fn upsert(&mut self, entry: RunCatalogEntry) -> Result<(), RunCatalogRepositoryError> {
        self.entries.insert(entry.run_id.0.clone(), entry);
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<RunCatalogEntry, RunCatalogRepositoryError> {
        self.entries
            .get(&run_id.0)
            .cloned()
            .ok_or_else(|| RunCatalogRepositoryError::NotFound {
                run_id: run_id.0.clone(),
            })
    }
}

#[derive(Debug, Default)]
pub struct InMemoryEventArchive {
    events: BTreeMap<String, Vec<RunEventEnvelope>>,
}

impl EventArchive for InMemoryEventArchive {
    fn append(&mut self, event: RunEventEnvelope) -> Result<(), EventArchiveError> {
        self.events
            .entry(event.run_id.0.clone())
            .or_default()
            .push(event);
        Ok(())
    }

    fn read(&self, query: EventArchiveQuery) -> Result<EventArchiveSlice, EventArchiveError> {
        let events =
            self.events
                .get(&query.run_id.0)
                .ok_or_else(|| EventArchiveError::NotFound {
                    run_id: query.run_id.0.clone(),
                })?;

        let mut selected = events
            .iter()
            .filter(|event| {
                query
                    .after_event_seq
                    .map(|after| event.event_seq > after)
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        let truncated = query
            .limit
            .map(|limit| selected.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit {
            selected.truncate(limit);
        }

        Ok(EventArchiveSlice {
            run_id: query.run_id,
            after_event_seq: query.after_event_seq,
            latest_event_seq: events.last().map(|event| event.event_seq),
            next_after_event_seq: selected.last().map(|event| event.event_seq),
            truncated,
            events: selected,
        })
    }
}

#[derive(Debug, Default)]
pub struct InMemoryRunWaitStateStore {
    wait_states: BTreeMap<String, RunWaitStateRecord>,
}

impl RunWaitStateStore for InMemoryRunWaitStateStore {
    fn upsert_wait_state(
        &mut self,
        wait_state: RunWaitStateRecord,
    ) -> Result<(), RunWaitStateStoreError> {
        self.wait_states
            .insert(wait_state.run_id.0.clone(), wait_state);
        Ok(())
    }

    fn load_wait_state(
        &self,
        run_id: &RunId,
    ) -> Result<RunWaitStateRecord, RunWaitStateStoreError> {
        self.wait_states
            .get(&run_id.0)
            .cloned()
            .ok_or_else(|| RunWaitStateStoreError::NotFound {
                run_id: run_id.0.clone(),
            })
    }

    fn clear_wait_state(&mut self, run_id: &RunId) -> Result<(), RunWaitStateStoreError> {
        self.wait_states.remove(&run_id.0);
        Ok(())
    }
}

pub type InMemorySignerStateStore = InMemoryRunWaitStateStore;

#[derive(Debug, Default)]
pub struct InMemoryRuntimeAuditArchive {
    audits: BTreeMap<String, Vec<RuntimeAuditRecord>>,
}

impl RuntimeAuditArchive for InMemoryRuntimeAuditArchive {
    fn append(&mut self, record: RuntimeAuditRecord) -> Result<(), RuntimeAuditArchiveError> {
        self.audits
            .entry(record.run_id.0.clone())
            .or_default()
            .push(record);
        Ok(())
    }

    fn read(
        &self,
        query: RuntimeAuditQuery,
    ) -> Result<RuntimeAuditSlice, RuntimeAuditArchiveError> {
        let records =
            self.audits
                .get(&query.run_id.0)
                .ok_or_else(|| RuntimeAuditArchiveError::NotFound {
                    run_id: query.run_id.0.clone(),
                })?;

        let mut selected = records
            .iter()
            .filter(|record| {
                query
                    .after_audit_seq
                    .map(|after| record.audit_seq > after)
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        let truncated = query
            .limit
            .map(|limit| selected.len() > limit)
            .unwrap_or(false);
        if let Some(limit) = query.limit {
            selected.truncate(limit);
        }

        Ok(RuntimeAuditSlice {
            run_id: query.run_id,
            after_audit_seq: query.after_audit_seq,
            latest_audit_seq: records.last().map(|record| record.audit_seq),
            next_after_audit_seq: selected.last().map(|record| record.audit_seq),
            truncated,
            records: selected,
        })
    }
}
