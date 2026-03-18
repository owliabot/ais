use ais_agent_control::{
    commands::{BeginRunCommand, InspectRunCommand},
    ids::RunId,
};
use ais_agent_host::{
    control::{HostAcceptedResponse, HostCommandOutcome, HostCommandResponse},
    session::{HostRunLink, HostSessionId},
};

use crate::runtime::{classify_validated_recovery_view, ActiveRun};

use super::super::{
    conversion, launch_spec, RuntimeHostService, RuntimeHostServiceError, RuntimeHostServiceResult,
};

impl<R, C, M, K, E, S, G, A, Q> RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateStore + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
    Q: crate::persistence::RunClaimRepository + Send,
{
    pub async fn begin_run(
        &mut self,
        host_session_id: HostSessionId,
        command: BeginRunCommand,
    ) -> RuntimeHostServiceResult {
        let run_seq = self.allocate_next_run_seq()?;
        let launch_spec = command.launch_spec.clone().ok_or_else(|| {
            RuntimeHostServiceError::invalid_command("begin_run requires launch_spec")
        })?;
        let mission = conversion::normalize_mission(command.mission, run_seq, &launch_spec);
        let run_id = RunId(format!("run-{run_seq}"));
        let mut checkpoint = conversion::initial_checkpoint(run_id.clone(), &mission);
        launch_spec::seed_launch_spec_checkpoint(
            &mut checkpoint,
            &self.execution_wiring,
            &launch_spec,
        )
        .map_err(RuntimeHostServiceError::invalid_command)?;
        let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
        runtime.record_command(command.command_id, None);

        let events =
            crate::events::RuntimeEventEmitter::emit_started(&mut runtime, "mission_accepted");
        self.persist_new_run(&run_id, &mission, &checkpoint, &runtime, &events)?;
        self.seed_initial_claim(&host_session_id, &run_id)?;
        self.session_store.link_run(HostRunLink::new(
            host_session_id.clone(),
            run_id.clone(),
            mission.goal.clone(),
            mission.allowed_chains.clone(),
        ));
        let session = self.session_store.session_snapshot(&host_session_id);

        Ok(HostCommandOutcome {
            response: HostCommandResponse::Accepted(HostAcceptedResponse {
                run_id: Some(run_id.clone()),
                message: Some("run created".to_owned()),
                session,
            }),
            events,
        })
    }

    pub async fn inspect_run(
        &mut self,
        host_session_id: HostSessionId,
        command: InspectRunCommand,
    ) -> RuntimeHostServiceResult {
        let (mission, checkpoint) = self.load_inspect_projection_input(&command.run_id)?;
        let recent_events =
            self.load_recent_event_tail(&command.run_id, Self::INSPECT_RECENT_EVENT_LIMIT)?;
        self.establish_inspect_session_link(&host_session_id, &command.run_id, &mission)?;
        let mut inspect =
            ais_agent_host::inspect::project_inspect_snapshot_with_recovery_and_events(
                &mission,
                &checkpoint,
                classify_validated_recovery_view(&checkpoint)
                    .map_err(RuntimeHostServiceError::InvalidRecoveryContract)?,
                &recent_events,
            );
        self.hydrate_inspect_ownership(&mut inspect)?;
        let _ = self.session_store.apply_inspect(&host_session_id, &inspect);
        Ok(HostCommandOutcome {
            response: HostCommandResponse::Inspect(inspect),
            events: Vec::new(),
        })
    }
}
