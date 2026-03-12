//! In-memory repository shells for hot tests and contract freeze coverage.

use std::collections::BTreeMap;

use ais_agent_control::{audit::RuntimeAuditRecord, events::RunEventEnvelope, ids::RunId};
use ais_agent_core::checkpoint::CheckpointSnapshot;
use ais_agent_core::mission::Mission;
use ais_agent_core::runtime::SignerRequestState;

use crate::persistence::{
    CheckpointArchive, CheckpointArchiveEntry, CheckpointArchiveError, EventArchive,
    EventArchiveError, EventArchiveQuery, EventArchiveSlice, MissionRepository,
    MissionRepositoryError, RunCatalogEntry, RunCatalogRepository, RunCatalogRepositoryError,
    RuntimeAuditArchive, RuntimeAuditArchiveError, RuntimeAuditQuery, RuntimeAuditSlice,
    SignerStateArchive, SignerStateArchiveError,
};

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
pub struct InMemorySignerStateArchive {
    signer_states: BTreeMap<String, SignerRequestState>,
}

impl SignerStateArchive for InMemorySignerStateArchive {
    fn upsert(&mut self, signer_state: SignerRequestState) -> Result<(), SignerStateArchiveError> {
        self.signer_states
            .insert(signer_state.run_id.0.clone(), signer_state);
        Ok(())
    }

    fn load(&self, run_id: &RunId) -> Result<SignerRequestState, SignerStateArchiveError> {
        self.signer_states.get(&run_id.0).cloned().ok_or_else(|| {
            SignerStateArchiveError::NotFound {
                run_id: run_id.0.clone(),
            }
        })
    }

    fn clear(&mut self, run_id: &RunId) -> Result<(), SignerStateArchiveError> {
        self.signer_states.remove(&run_id.0);
        Ok(())
    }
}

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
