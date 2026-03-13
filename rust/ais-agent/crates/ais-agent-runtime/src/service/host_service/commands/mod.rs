mod begin_inspect;
mod mutation;
mod ownership;

use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandResponse},
    inspect::{project_inspect_snapshot_with_recovery, project_pause_bundle_with_recovery},
    session::HostSessionId,
};

use crate::runtime::{classify_validated_recovery_view, ActiveRun};

use super::{RuntimeHostService, RuntimeHostServiceError};

impl<R, C, M, K, E, S, G, A, Q> RuntimeHostService<R, C, M, K, E, S, G, A, Q>
where
    R: crate::runtime::RunRepository + Send,
    C: crate::persistence::CheckpointRepository + Send,
    M: crate::persistence::MissionRepository + Send,
    K: crate::persistence::RunCatalogRepository + Send,
    E: crate::persistence::EventArchive + Send,
    S: ais_agent_host::session::HostSessionStore + Send,
    G: crate::persistence::SignerStateArchive + Send,
    A: crate::persistence::RuntimeAuditArchive + Send,
    Q: crate::persistence::RunClaimRepository + Send,
{
    fn apply_runtime_inspect_to_session(
        &mut self,
        host_session_id: &HostSessionId,
        runtime: &ActiveRun,
    ) -> Result<ais_agent_host::inspect::InspectSnapshot, RuntimeHostServiceError> {
        let mut inspect = project_inspect_snapshot_with_recovery(
            &runtime.mission,
            &runtime.checkpoint,
            classify_validated_recovery_view(&runtime.checkpoint)
                .map_err(RuntimeHostServiceError::InvalidRecoveryContract)?,
        );
        self.hydrate_inspect_ownership(&mut inspect)?;
        let _ = self.session_store.apply_inspect(host_session_id, &inspect);
        Ok(inspect)
    }

    fn inspect_or_pause_response(
        &mut self,
        host_session_id: &HostSessionId,
        runtime: &ActiveRun,
    ) -> Result<HostCommandResponse, RuntimeHostServiceError> {
        let recovery = classify_validated_recovery_view(&runtime.checkpoint)
            .map_err(RuntimeHostServiceError::InvalidRecoveryContract)?;
        let mut inspect = project_inspect_snapshot_with_recovery(
            &runtime.mission,
            &runtime.checkpoint,
            recovery.clone(),
        );
        self.hydrate_inspect_ownership(&mut inspect)?;
        let _ = self.session_store.apply_inspect(host_session_id, &inspect);

        Ok(
            if let Some(mut pause) =
                project_pause_bundle_with_recovery(&runtime.checkpoint, recovery)
            {
                self.hydrate_pause_ownership(&mut pause)?;
                HostCommandResponse::Pause(pause)
            } else {
                HostCommandResponse::Inspect(inspect)
            },
        )
    }

    fn inspect_outcome(
        &mut self,
        host_session_id: &HostSessionId,
        runtime: &ActiveRun,
        events: Vec<ais_agent_control::events::RunEventEnvelope>,
    ) -> Result<HostCommandOutcome, RuntimeHostServiceError> {
        let inspect = self.apply_runtime_inspect_to_session(host_session_id, runtime)?;
        Ok(HostCommandOutcome {
            response: HostCommandResponse::Inspect(inspect),
            events,
        })
    }
}
