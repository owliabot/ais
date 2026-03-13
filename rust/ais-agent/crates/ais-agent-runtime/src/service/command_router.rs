//! Command routing for the runtime host service.

use std::{future::Future, pin::Pin};

use ais_agent_control::commands::RunCommand;
use ais_agent_host::session::HostedRunCommand;

use crate::service::host_service::{
    RuntimeHostService, RuntimeHostServiceError, RuntimeHostServiceResult,
};

#[derive(Debug, Default)]
pub struct RuntimeCommandRouter;

impl RuntimeCommandRouter {
    pub fn route<R, C, M, K, E, S, G, A, Q>(
        service: &mut RuntimeHostService<R, C, M, K, E, S, G, A, Q>,
        envelope: HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = RuntimeHostServiceResult> + Send + '_>>
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
        Box::pin(async move {
            let host_session_id = envelope.host_session_id;
            let command = envelope.command;

            match command {
                RunCommand::BeginRun(command) => service.begin_run(host_session_id, command).await,
                RunCommand::InspectRun(command) => {
                    service.inspect_run(host_session_id, command).await
                }
                RunCommand::ClaimRun(command) => service.claim_run(host_session_id, command).await,
                RunCommand::RenewRunClaim(command) => {
                    service.renew_run_claim(host_session_id, command).await
                }
                RunCommand::ReleaseRunClaim(command) => {
                    service.release_run_claim(host_session_id, command).await
                }
                RunCommand::StepRun(command) => service.step_run(host_session_id, command).await,
                RunCommand::SubmitEvidence(command) => {
                    service.submit_evidence(host_session_id, command).await
                }
                RunCommand::SubmitEnvelope(command) => {
                    service.submit_envelope(host_session_id, command).await
                }
                RunCommand::SubmitSignerDecision(command) => {
                    service
                        .submit_signer_decision(host_session_id, command)
                        .await
                }
                RunCommand::SubmitPlanPatch(command) => {
                    service.submit_plan_patch(host_session_id, command).await
                }
                RunCommand::RequestCancelRun(command) => {
                    service.request_cancel_run(host_session_id, command).await
                }
                RunCommand::CancelRun(command) => {
                    service.cancel_run(host_session_id, command).await
                }
            }
        })
    }

    pub fn into_outcome(
        result: Result<ais_agent_host::control::HostCommandOutcome, RuntimeHostServiceError>,
    ) -> ais_agent_host::control::HostCommandOutcome {
        match result {
            Ok(outcome) => outcome,
            Err(error) => error.into_outcome(),
        }
    }
}
