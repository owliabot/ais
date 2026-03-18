use ais_agent_control::{
    commands::{ClaimRunCommand, ReleaseRunClaimCommand, RenewRunClaimCommand},
    ownership::{OwnershipErrorCode, RunClaimStatus},
};
use ais_agent_host::{control::HostCommandOutcome, session::HostSessionId};
use tracing::info;

use super::super::{RuntimeHostService, RuntimeHostServiceError, RuntimeHostServiceResult};

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
    pub async fn claim_run(
        &mut self,
        host_session_id: HostSessionId,
        command: ClaimRunCommand,
    ) -> RuntimeHostServiceResult {
        let (mission, checkpoint) = self.load_inspect_projection_input(&command.run_id)?;
        let policy = self.claim_policy(&command.run_id)?;

        if !policy.claim_required_for_mutation
            && matches!(
                command.mode,
                ais_agent_control::ownership::RunClaimMode::ExclusiveMutation
            )
        {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimTransferRequired,
                run_id: command.run_id.0.clone(),
                message: "terminal runs do not accept new exclusive mutation claims".to_owned(),
            });
        }

        let maybe_expired = self.expire_stale_claim_if_needed(&host_session_id, &command.run_id)?;
        let current = match maybe_expired {
            Some(expired) => Some(expired),
            None => self.load_effective_claim(&command.run_id)?,
        };

        let (acquired, lifecycle_event, superseded_claim_id) = match current {
            None => (
                self.acquire_claim(
                    &host_session_id,
                    self.build_claim(
                        &host_session_id,
                        &command.run_id,
                        command.owner_kind,
                        command.owner_instance_id,
                        command.mode,
                        command.requested_lease_ms,
                    ),
                )?,
                None,
                None,
            ),
            Some(current) if current.status == RunClaimStatus::Expired => (
                self.acquire_claim(
                    &host_session_id,
                    self.build_claim(
                        &host_session_id,
                        &command.run_id,
                        command.owner_kind,
                        command.owner_instance_id,
                        command.mode,
                        command.requested_lease_ms,
                    ),
                )?,
                None,
                None,
            ),
            Some(current)
                if current.host_session_id == host_session_id.0
                    && current.owner_instance_id == command.owner_instance_id
                    && current.mode == command.mode =>
            {
                (current, None, None)
            }
            Some(current) => {
                if !command.allow_supersede {
                    return Err(RuntimeHostServiceError::OwnershipViolation {
                        code: OwnershipErrorCode::ClaimConflict,
                        run_id: command.run_id.0.clone(),
                        message: format!(
                            "active claim `{}` already exists for run `{}`",
                            current.claim_id.0, command.run_id.0
                        ),
                    });
                }
                if !policy.allow_supersede_active_claim {
                    return Err(RuntimeHostServiceError::OwnershipViolation {
                        code: OwnershipErrorCode::ClaimTransferRequired,
                        run_id: command.run_id.0.clone(),
                        message: "run state does not allow active-claim supersede".to_owned(),
                    });
                }
                let expected_claim_id =
                    command.expected_current_claim_id.clone().ok_or_else(|| {
                        RuntimeHostServiceError::OwnershipViolation {
                            code: OwnershipErrorCode::ClaimTransferRequired,
                            run_id: command.run_id.0.clone(),
                            message: "active-claim supersede requires expected_current_claim_id"
                                .to_owned(),
                        }
                    })?;
                let expected_claim_epoch =
                    command.expected_current_claim_epoch.ok_or_else(|| {
                        RuntimeHostServiceError::OwnershipViolation {
                            code: OwnershipErrorCode::ClaimTransferRequired,
                            run_id: command.run_id.0.clone(),
                            message: "active-claim supersede requires expected_current_claim_epoch"
                                .to_owned(),
                        }
                    })?;

                if current.claim_id != expected_claim_id {
                    return Err(RuntimeHostServiceError::OwnershipViolation {
                        code: OwnershipErrorCode::ClaimConflict,
                        run_id: command.run_id.0.clone(),
                        message: format!(
                            "expected active claim `{}`, found `{}`",
                            expected_claim_id.0, current.claim_id.0
                        ),
                    });
                }
                if current.claim_epoch != expected_claim_epoch {
                    return Err(RuntimeHostServiceError::OwnershipViolation {
                        code: OwnershipErrorCode::ClaimEpochStale,
                        run_id: command.run_id.0.clone(),
                        message: format!(
                            "expected claim epoch `{expected_claim_epoch}`, found `{}`",
                            current.claim_epoch
                        ),
                    });
                }

                let predecessor_claim_id = current.claim_id.0.clone();
                (
                    self.claim_repo
                        .supersede(crate::persistence::ClaimSupersedeRequest {
                            run_id: command.run_id.clone(),
                            predecessor_claim_id: current.claim_id.clone(),
                            predecessor_claim_epoch: current.claim_epoch,
                            successor_claim: self.build_claim(
                                &host_session_id,
                                &command.run_id,
                                command.owner_kind,
                                command.owner_instance_id,
                                command.mode,
                                command.requested_lease_ms,
                            ),
                        })
                        .map(|result| result.successor)
                        .map_err(|error| RuntimeHostServiceError::OwnershipViolation {
                            code: OwnershipErrorCode::ClaimTransferRequired,
                            run_id: command.run_id.0.clone(),
                            message: error.to_string(),
                        })?,
                    Some("runtime.host.claim_superseded"),
                    Some(predecessor_claim_id),
                )
            }
        };
        if let Some(event) = lifecycle_event {
            info!(
                run_id = %acquired.run_id.0,
                host_session_id = %host_session_id.0,
                claim_id = %acquired.claim_id.0,
                claim_epoch = acquired.claim_epoch,
                superseded_claim_id = superseded_claim_id.as_deref(),
                "{}", event
            );
        }

        self.force_session_link(&host_session_id, &command.run_id, &mission);
        let runtime = self.load_hot_runtime(&command.run_id)?;
        if let Some(runtime) = runtime.as_ref() {
            return Ok(HostCommandOutcome {
                response: self.inspect_or_pause_response(&host_session_id, runtime)?,
                events: Vec::new(),
            });
        }

        let mut inspect = ais_agent_host::inspect::project_inspect_snapshot_with_recovery(
            &mission,
            &checkpoint,
            crate::runtime::classify_validated_recovery_view(&checkpoint)
                .map_err(RuntimeHostServiceError::InvalidRecoveryContract)?,
        );
        inspect.ownership.current_claim = Some(acquired);
        let _ = self.session_store.apply_inspect(&host_session_id, &inspect);
        Ok(HostCommandOutcome {
            response: ais_agent_host::control::HostCommandResponse::Inspect(inspect),
            events: Vec::new(),
        })
    }

    pub async fn renew_run_claim(
        &mut self,
        host_session_id: HostSessionId,
        command: RenewRunClaimCommand,
    ) -> RuntimeHostServiceResult {
        let (mission, _) = self.load_inspect_projection_input(&command.run_id)?;
        let current = self.load_effective_claim(&command.run_id)?.ok_or_else(|| {
            RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimRequired,
                run_id: command.run_id.0.clone(),
                message: "run has no active claim to renew".to_owned(),
            }
        })?;
        if current.status == RunClaimStatus::Expired {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimExpired,
                run_id: command.run_id.0.clone(),
                message: format!("claim `{}` has expired", current.claim_id.0),
            });
        }
        if current.host_session_id != host_session_id.0 {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimNotOwner,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "active claim `{}` belongs to session `{}`",
                    current.claim_id.0, current.host_session_id
                ),
            });
        }
        if current.claim_id != command.claim_id {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimConflict,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "active claim `{}` does not match requested claim `{}`",
                    current.claim_id.0, command.claim_id.0
                ),
            });
        }
        if current.claim_epoch != command.claim_epoch {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimEpochStale,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "expected claim epoch `{}`, found `{}`",
                    command.claim_epoch, current.claim_epoch
                ),
            });
        }

        let renewed = self
            .claim_repo
            .renew(crate::persistence::ClaimRenewRequest {
                run_id: command.run_id.clone(),
                claim_id: command.claim_id,
                claim_epoch: command.claim_epoch,
                renewed_at_ms: super::super::RuntimeHostService::<R, C, M, K, E, S, G, A, Q>::claim_now_ms(),
                lease_expires_at_ms: Some(
                    super::super::RuntimeHostService::<R, C, M, K, E, S, G, A, Q>::claim_now_ms()
                        .saturating_add(
                            super::super::RuntimeHostService::<
                                R,
                                C,
                                M,
                                K,
                                E,
                                S,
                                G,
                                A,
                                Q,
                            >::requested_claim_lease_ms(command.requested_lease_ms),
                        ),
                ),
            })
            .map_err(|error| RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimTransferRequired,
                run_id: command.run_id.0.clone(),
                message: error.to_string(),
            })?;
        info!(
            run_id = %renewed.run_id.0,
            host_session_id = %host_session_id.0,
            claim_id = %renewed.claim_id.0,
            claim_epoch = renewed.claim_epoch,
            "runtime.host.claim_renewed"
        );

        self.force_session_link(&host_session_id, &command.run_id, &mission);
        let runtime = self.load_hot_runtime(&command.run_id)?;
        if let Some(runtime) = runtime.as_ref() {
            self.inspect_outcome(&host_session_id, runtime, Vec::new())
        } else {
            self.inspect_run(
                host_session_id,
                ais_agent_control::commands::InspectRunCommand {
                    command_id: command.command_id,
                    run_id: command.run_id,
                },
            )
            .await
        }
    }

    pub async fn release_run_claim(
        &mut self,
        host_session_id: HostSessionId,
        command: ReleaseRunClaimCommand,
    ) -> RuntimeHostServiceResult {
        let (mission, _) = self.load_inspect_projection_input(&command.run_id)?;
        let policy = self.claim_policy(&command.run_id)?;
        let current = self.load_effective_claim(&command.run_id)?.ok_or_else(|| {
            RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimRequired,
                run_id: command.run_id.0.clone(),
                message: "run has no active claim to release".to_owned(),
            }
        })?;
        if current.status == RunClaimStatus::Expired {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimExpired,
                run_id: command.run_id.0.clone(),
                message: format!("claim `{}` has expired", current.claim_id.0),
            });
        }
        if current.host_session_id != host_session_id.0 {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimNotOwner,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "active claim `{}` belongs to session `{}`",
                    current.claim_id.0, current.host_session_id
                ),
            });
        }
        if current.claim_id != command.claim_id {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimConflict,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "active claim `{}` does not match requested claim `{}`",
                    current.claim_id.0, command.claim_id.0
                ),
            });
        }
        if current.claim_epoch != command.claim_epoch {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimEpochStale,
                run_id: command.run_id.0.clone(),
                message: format!(
                    "expected claim epoch `{}`, found `{}`",
                    command.claim_epoch, current.claim_epoch
                ),
            });
        }
        if !policy.allow_release {
            return Err(RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimTransferRequired,
                run_id: command.run_id.0.clone(),
                message: "run state does not allow active-claim release".to_owned(),
            });
        }

        let released = self
            .claim_repo
            .release(crate::persistence::ClaimReleaseRequest {
                run_id: command.run_id.clone(),
                claim_id: command.claim_id,
                claim_epoch: command.claim_epoch,
            })
            .map_err(|error| RuntimeHostServiceError::OwnershipViolation {
                code: OwnershipErrorCode::ClaimTransferRequired,
                run_id: command.run_id.0.clone(),
                message: error.to_string(),
            })?;
        info!(
            run_id = %released.run_id.0,
            host_session_id = %host_session_id.0,
            claim_id = %released.claim_id.0,
            claim_epoch = released.claim_epoch,
            "runtime.host.claim_released"
        );

        self.force_session_link(&host_session_id, &command.run_id, &mission);
        let runtime = self.load_hot_runtime(&command.run_id)?;
        if let Some(runtime) = runtime.as_ref() {
            self.inspect_outcome(&host_session_id, runtime, Vec::new())
        } else {
            self.inspect_run(
                host_session_id,
                ais_agent_control::commands::InspectRunCommand {
                    command_id: command.command_id,
                    run_id: command.run_id,
                },
            )
            .await
        }
    }
}
