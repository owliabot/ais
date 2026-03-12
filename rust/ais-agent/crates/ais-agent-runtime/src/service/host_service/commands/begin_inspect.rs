use ais_agent_control::{
    commands::{BeginRunCommand, InspectRunCommand},
    ids::RunId,
};
use ais_agent_host::{
    control::{HostAcceptedResponse, HostCommandOutcome, HostCommandResponse},
    inspect::project_inspect_snapshot_with_recovery,
    session::{HostRunLink, HostSessionId},
};

use crate::runtime::{classify_validated_recovery_view, ActiveRun};

use super::super::{
    conversion, RuntimeHostService, RuntimeHostServiceError, RuntimeHostServiceResult,
};

impl<R, C, M, K, E, S, G, A> RuntimeHostService<R, C, M, K, E, S, G, A>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateArchive + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
{
    pub async fn begin_run(
        &mut self,
        host_session_id: HostSessionId,
        command: BeginRunCommand,
    ) -> RuntimeHostServiceResult {
        let run_seq = self.allocate_next_run_seq()?;
        let mission = conversion::normalize_mission(command.mission, run_seq);
        let run_id = RunId(format!("run-{run_seq}"));
        let checkpoint = conversion::initial_checkpoint(run_id.clone(), &mission);
        let mut runtime = ActiveRun::new(mission.clone(), checkpoint.clone());
        runtime.record_command(command.command_id, None);

        let events =
            crate::events::RuntimeEventEmitter::emit_started(&mut runtime, "mission_accepted");
        self.persist_new_run(&run_id, &mission, &checkpoint, &runtime, &events)?;
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
        self.establish_inspect_session_link(&host_session_id, &command.run_id, &mission)?;
        let inspect = project_inspect_snapshot_with_recovery(
            &mission,
            &checkpoint,
            classify_validated_recovery_view(&checkpoint)
                .map_err(RuntimeHostServiceError::InvalidRecoveryContract)?,
        );
        let _ = self.session_store.apply_inspect(&host_session_id, &inspect);
        Ok(HostCommandOutcome {
            response: HostCommandResponse::Inspect(inspect),
            events: Vec::new(),
        })
    }
}
