use ais_agent_control::{
    commands::{ExpectedRuntimeVersion, SubmitPlanPatchCommand},
    patch::{PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchLegalityErrorCode {
    EmptyPatchId,
    EmptyOperations,
    MissingExpectedVersion,
    ExpectedVersionMismatch,
    RunIdMismatch,
    EmptyTargetRefs,
    InvalidFragmentPayload,
    InvalidEffectContractPayload,
    EmptyConstraintTightening,
    HistoryRewriteForbidden,
    EffectContractPreservationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct PatchLegalityError {
    pub code: PatchLegalityErrorCode,
    pub message: String,
    pub operation_index: Option<usize>,
}

pub fn validate_submit_plan_patch_command(
    command: &SubmitPlanPatchCommand,
) -> Result<(), PatchLegalityError> {
    validate_plan_patch_submission(&command.patch)?;

    if command.run_id != command.patch.run_id {
        return Err(PatchLegalityError {
            code: PatchLegalityErrorCode::RunIdMismatch,
            message: "submit_plan_patch run_id must match patch.run_id".to_owned(),
            operation_index: None,
        });
    }

    let Some(expected) = command.expected_version.as_ref() else {
        return Err(PatchLegalityError {
            code: PatchLegalityErrorCode::MissingExpectedVersion,
            message: "submit_plan_patch requires expected_version".to_owned(),
            operation_index: None,
        });
    };

    validate_expected_version(expected, command)
}

pub fn validate_plan_patch_submission(
    patch: &PlanPatchSubmission,
) -> Result<(), PatchLegalityError> {
    if patch.patch_id.trim().is_empty() {
        return Err(PatchLegalityError {
            code: PatchLegalityErrorCode::EmptyPatchId,
            message: "plan patch must have a non-empty patch_id".to_owned(),
            operation_index: None,
        });
    }

    if patch.operations.is_empty() {
        return Err(PatchLegalityError {
            code: PatchLegalityErrorCode::EmptyOperations,
            message: "plan patch must include at least one operation".to_owned(),
            operation_index: None,
        });
    }

    match &patch.target {
        PlanPatchTarget::ActiveFrontier => {}
        PlanPatchTarget::NodeSet { node_ids } | PlanPatchTarget::FailedFragment { node_ids } => {
            if node_ids.is_empty() {
                return Err(PatchLegalityError {
                    code: PatchLegalityErrorCode::EmptyTargetRefs,
                    message: "node-targeted plan patches must include node ids".to_owned(),
                    operation_index: None,
                });
            }
        }
        PlanPatchTarget::PendingVerifyBranch { effect_refs } => {
            if effect_refs.is_empty() {
                return Err(PatchLegalityError {
                    code: PatchLegalityErrorCode::EmptyTargetRefs,
                    message: "pending_verify_branch patches must include effect refs".to_owned(),
                    operation_index: None,
                });
            }
        }
    }

    let mut preserves_or_replaces_effect_contract = false;

    for (index, operation) in patch.operations.iter().enumerate() {
        match operation {
            PlanPatchOperation::ReplaceFragment {
                fragment,
                preserved_effect_refs,
            }
            | PlanPatchOperation::AppendFragment {
                fragment,
                preserved_effect_refs,
            } => {
                if !fragment.is_object() {
                    return Err(PatchLegalityError {
                        code: PatchLegalityErrorCode::InvalidFragmentPayload,
                        message: "fragment patch operations require object payloads".to_owned(),
                        operation_index: Some(index),
                    });
                }
                if !preserved_effect_refs.is_empty() {
                    preserves_or_replaces_effect_contract = true;
                }
            }
            PlanPatchOperation::DropBranch { node_ids } => {
                if node_ids.is_empty() {
                    return Err(PatchLegalityError {
                        code: PatchLegalityErrorCode::EmptyTargetRefs,
                        message: "drop_branch operations must include node ids".to_owned(),
                        operation_index: Some(index),
                    });
                }
                if matches!(patch.target, PlanPatchTarget::PendingVerifyBranch { .. }) {
                    return Err(PatchLegalityError {
                        code: PatchLegalityErrorCode::HistoryRewriteForbidden,
                        message: "pending_verify_branch patches cannot drop already-linked verify ancestry".to_owned(),
                        operation_index: Some(index),
                    });
                }
            }
            PlanPatchOperation::TightenConstraints { constraints } => {
                if constraints.is_empty() {
                    return Err(PatchLegalityError {
                        code: PatchLegalityErrorCode::EmptyConstraintTightening,
                        message:
                            "tighten_constraints operations must include at least one constraint"
                                .to_owned(),
                        operation_index: Some(index),
                    });
                }
            }
            PlanPatchOperation::ReplaceEffectContract {
                effect_ref,
                contract,
            } => {
                if effect_ref.trim().is_empty() || !contract.is_object() {
                    return Err(PatchLegalityError {
                        code: PatchLegalityErrorCode::InvalidEffectContractPayload,
                        message:
                            "replace_effect_contract requires a non-empty effect_ref and object payload"
                                .to_owned(),
                        operation_index: Some(index),
                    });
                }
                preserves_or_replaces_effect_contract = true;
            }
        }
    }

    if matches!(patch.target, PlanPatchTarget::PendingVerifyBranch { .. })
        && !preserves_or_replaces_effect_contract
        && patch
            .expected_outcome
            .as_ref()
            .is_none_or(|outcome| outcome.preserved_effect_refs.is_empty())
    {
        return Err(PatchLegalityError {
            code: PatchLegalityErrorCode::EffectContractPreservationRequired,
            message: "pending_verify_branch patches must preserve or replace effect contracts"
                .to_owned(),
            operation_index: None,
        });
    }

    Ok(())
}

fn validate_expected_version(
    expected: &ExpectedRuntimeVersion,
    command: &SubmitPlanPatchCommand,
) -> Result<(), PatchLegalityError> {
    let checkpoint_seq_matches =
        expected.checkpoint_seq == Some(command.patch.basis_checkpoint_seq);
    let plan_epoch_matches = expected.plan_epoch == Some(command.patch.basis_plan_epoch);

    if checkpoint_seq_matches && plan_epoch_matches {
        return Ok(());
    }

    Err(PatchLegalityError {
        code: PatchLegalityErrorCode::ExpectedVersionMismatch,
        message:
            "submit_plan_patch expected_version must match patch basis_checkpoint_seq and basis_plan_epoch"
                .to_owned(),
        operation_index: None,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ais_agent_control::{
        commands::{ExpectedRuntimeVersion, SubmitPlanPatchCommand},
        ids::{CommandId, RunId},
        patch::{PatchOutcome, PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget},
        recovery::{RecoveryDisposition, RunFailureCode},
    };
    use serde_json::json;

    use super::{
        validate_plan_patch_submission, validate_submit_plan_patch_command, PatchLegalityErrorCode,
    };

    #[test]
    fn submit_plan_patch_requires_matching_expected_version() {
        let command = sample_command();

        assert!(validate_submit_plan_patch_command(&command).is_ok());

        let mut stale = command.clone();
        stale.expected_version = Some(ExpectedRuntimeVersion {
            checkpoint_seq: Some(7),
            plan_epoch: Some(1),
        });

        let error = validate_submit_plan_patch_command(&stale).expect_err("stale basis");
        assert_eq!(error.code, PatchLegalityErrorCode::ExpectedVersionMismatch);
    }

    #[test]
    fn pending_verify_branch_requires_effect_contract_preservation() {
        let mut patch = sample_patch();
        patch.target = PlanPatchTarget::PendingVerifyBranch {
            effect_refs: vec!["effect.swap".to_owned()],
        };
        patch.operations = vec![PlanPatchOperation::AppendFragment {
            fragment: json!({"nodes": {}}),
            preserved_effect_refs: Vec::new(),
        }];
        patch.expected_outcome = None;

        let error = validate_plan_patch_submission(&patch).expect_err("missing effect linkage");
        assert_eq!(
            error.code,
            PatchLegalityErrorCode::EffectContractPreservationRequired
        );
    }

    #[test]
    fn pending_verify_branch_cannot_drop_branch_history() {
        let mut patch = sample_patch();
        patch.target = PlanPatchTarget::PendingVerifyBranch {
            effect_refs: vec!["effect.swap".to_owned()],
        };
        patch.operations = vec![PlanPatchOperation::DropBranch {
            node_ids: vec!["verify.swap".to_owned()],
        }];

        let error = validate_plan_patch_submission(&patch).expect_err("history rewrite");
        assert_eq!(error.code, PatchLegalityErrorCode::HistoryRewriteForbidden);
    }

    #[test]
    fn node_set_target_requires_node_refs() {
        let mut patch = sample_patch();
        patch.target = PlanPatchTarget::NodeSet {
            node_ids: Vec::new(),
        };

        let error = validate_plan_patch_submission(&patch).expect_err("missing target refs");
        assert_eq!(error.code, PatchLegalityErrorCode::EmptyTargetRefs);
    }

    fn sample_command() -> SubmitPlanPatchCommand {
        SubmitPlanPatchCommand {
            command_id: CommandId("cmd-patch".to_owned()),
            run_id: RunId("run-1".to_owned()),
            patch: sample_patch(),
            expected_version: Some(ExpectedRuntimeVersion {
                checkpoint_seq: Some(8),
                plan_epoch: Some(2),
            }),
        }
    }

    fn sample_patch() -> PlanPatchSubmission {
        PlanPatchSubmission {
            patch_id: "patch-1".to_owned(),
            run_id: RunId("run-1".to_owned()),
            basis_checkpoint_seq: 8,
            basis_plan_epoch: 2,
            reason_code: RunFailureCode::GovernorDenied,
            target: PlanPatchTarget::FailedFragment {
                node_ids: vec!["govern.swap".to_owned()],
            },
            operations: vec![
                PlanPatchOperation::ReplaceFragment {
                    fragment: json!({"nodes": {}, "roots": ["replan.swap"], "terminals": ["replan.swap"]}),
                    preserved_effect_refs: vec!["effect.swap".to_owned()],
                },
                PlanPatchOperation::TightenConstraints {
                    constraints: BTreeMap::from([("slippage_bps".to_owned(), json!(50))]),
                },
            ],
            expected_outcome: Some(PatchOutcome {
                next_recovery_disposition: Some(RecoveryDisposition::RetryReady),
                touched_node_refs: vec!["govern.swap".to_owned()],
                preserved_effect_refs: vec!["effect.swap".to_owned()],
            }),
        }
    }
}
