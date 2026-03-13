use std::collections::BTreeMap;

use ais_agent_control::ids::{ClaimId, CommandId, IdempotencyKey, RunId};

use crate::{
    control::HostCommandOutcome,
    inspect::InspectSnapshot,
    session::{
        HostRunLink, HostSessionId, HostSessionSnapshot, IdempotencyRecord, IdempotencyResolution,
    },
};

pub trait HostSessionStore {
    fn link_run(&mut self, link: HostRunLink);
    fn session_snapshot(&self, host_session_id: &HostSessionId) -> Option<HostSessionSnapshot>;
    fn run_link(&self, run_id: &RunId) -> Option<HostRunLink>;
    fn apply_inspect(
        &mut self,
        host_session_id: &HostSessionId,
        inspect: &InspectSnapshot,
    ) -> Option<HostRunLink>;
    fn register_idempotency(
        &mut self,
        host_session_id: HostSessionId,
        key: IdempotencyKey,
        command_id: CommandId,
        run_id: Option<RunId>,
        replay_claim_id: Option<ClaimId>,
    ) -> IdempotencyResolution;
    fn complete_idempotency(
        &mut self,
        host_session_id: &HostSessionId,
        key: &IdempotencyKey,
        outcome: HostCommandOutcome,
        run_id: Option<RunId>,
        replay_claim_id: Option<ClaimId>,
    );
    fn clear_idempotency(&mut self, host_session_id: &HostSessionId, key: &IdempotencyKey);
}

#[derive(Debug, Default)]
pub struct InMemoryHostSessionStore {
    runs_by_session: BTreeMap<String, Vec<RunId>>,
    links_by_run: BTreeMap<String, HostRunLink>,
    idempotency: BTreeMap<(String, String), IdempotencyRecord>,
}

impl HostSessionStore for InMemoryHostSessionStore {
    fn link_run(&mut self, link: HostRunLink) {
        let session_key = link.host_session_id.0.clone();
        let run_id = link.run_id.clone();
        if let Some(existing) = self.links_by_run.get(&run_id.0) {
            if existing.host_session_id != link.host_session_id {
                if let Some(previous_session_runs) =
                    self.runs_by_session.get_mut(&existing.host_session_id.0)
                {
                    previous_session_runs.retain(|existing_run_id| existing_run_id != &run_id);
                    if previous_session_runs.is_empty() {
                        self.runs_by_session.remove(&existing.host_session_id.0);
                    }
                }
            }
        }
        self.links_by_run.insert(run_id.0.clone(), link);
        let linked_runs = self.runs_by_session.entry(session_key).or_default();
        if !linked_runs.iter().any(|existing| existing == &run_id) {
            linked_runs.push(run_id);
        }
    }

    fn session_snapshot(&self, host_session_id: &HostSessionId) -> Option<HostSessionSnapshot> {
        let run_ids = self.runs_by_session.get(&host_session_id.0)?;
        let linked_runs: Vec<HostRunLink> = run_ids
            .iter()
            .filter_map(|run_id| self.links_by_run.get(&run_id.0))
            .filter(|link| &link.host_session_id == host_session_id)
            .cloned()
            .collect();
        let active_run_id = linked_runs.last().map(|link| link.run_id.clone());

        Some(HostSessionSnapshot {
            host_session_id: host_session_id.clone(),
            active_run_id,
            linked_runs,
        })
    }

    fn run_link(&self, run_id: &RunId) -> Option<HostRunLink> {
        self.links_by_run.get(&run_id.0).cloned()
    }

    fn apply_inspect(
        &mut self,
        host_session_id: &HostSessionId,
        inspect: &InspectSnapshot,
    ) -> Option<HostRunLink> {
        let link = self.links_by_run.get_mut(&inspect.run_id.0)?;
        if &link.host_session_id != host_session_id {
            return None;
        }
        link.apply_inspect(inspect);
        Some(link.clone())
    }

    fn register_idempotency(
        &mut self,
        host_session_id: HostSessionId,
        key: IdempotencyKey,
        command_id: CommandId,
        run_id: Option<RunId>,
        replay_claim_id: Option<ClaimId>,
    ) -> IdempotencyResolution {
        let composite_key = (host_session_id.0.clone(), key.0.clone());

        match self.idempotency.get(&composite_key) {
            None => {
                self.idempotency.insert(
                    composite_key,
                    IdempotencyRecord {
                        host_session_id,
                        key,
                        command_id,
                        run_id,
                        replay_claim_id,
                        outcome: None,
                    },
                );
                IdempotencyResolution::Accepted
            }
            Some(existing)
                if existing.command_id == command_id
                    && existing.replay_claim_id == replay_claim_id
                    && (existing.run_id == run_id || run_id.is_none()) =>
            {
                IdempotencyResolution::Replay {
                    existing_command_id: existing.command_id.clone(),
                    run_id: existing.run_id.clone(),
                    outcome: existing.outcome.clone(),
                }
            }
            Some(existing) => IdempotencyResolution::Conflict {
                existing_command_id: existing.command_id.clone(),
                run_id: existing.run_id.clone(),
            },
        }
    }

    fn complete_idempotency(
        &mut self,
        host_session_id: &HostSessionId,
        key: &IdempotencyKey,
        outcome: HostCommandOutcome,
        run_id: Option<RunId>,
        replay_claim_id: Option<ClaimId>,
    ) {
        if let Some(record) = self
            .idempotency
            .get_mut(&(host_session_id.0.clone(), key.0.clone()))
        {
            record.run_id = run_id;
            record.replay_claim_id = replay_claim_id;
            record.outcome = Some(outcome);
        }
    }

    fn clear_idempotency(&mut self, host_session_id: &HostSessionId, key: &IdempotencyKey) {
        self.idempotency
            .remove(&(host_session_id.0.clone(), key.0.clone()));
    }
}
