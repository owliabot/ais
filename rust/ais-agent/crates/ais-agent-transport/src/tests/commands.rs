use std::collections::BTreeMap;

use serde_json::json;

use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, ClaimRunCommand, EnvelopeKind, EnvelopeSubmission,
        ExpectedRuntimeVersion, InspectRunCommand, MissionSubmission, ReleaseRunClaimCommand,
        RenewRunClaimCommand, RequestCancelRunCommand, RunCommand, SignerDecisionKind,
        SignerDecisionSubmission, StepBudget, StepRunCommand, StepUntil, SubmitEnvelopeCommand,
        SubmitEvidenceCommand, SubmitPlanPatchCommand, SubmitSignerDecisionCommand,
    },
    ids::{ClaimId, CommandId, IdempotencyKey, RunId, SignerRequestId},
    ownership::{RunClaimMode, RunClaimOwnerKind},
    patch::{PlanPatchOperation, PlanPatchSubmission, PlanPatchTarget},
    recovery::RunFailureCode,
};
use ais_agent_host::session::{HostSessionId, HostedRunCommand};

pub fn sample_begin_command() -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some("request-begin".into()),
        command: RunCommand::BeginRun(BeginRunCommand {
            command_id: CommandId("cmd-begin".to_owned()),
            idempotency_key: IdempotencyKey("idem-begin".to_owned()),
            mission: MissionSubmission {
                goal: "swap usdc to eth".to_owned(),
                allowed_chains: vec!["eip155:1".to_owned()],
                constraints: Default::default(),
                budget: None,
                metadata: Default::default(),
            },
        }),
    }
}

pub fn inspect_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    inspect_command_for_session(run_id, request_id, "session-1")
}

pub fn inspect_command_for_session(
    run_id: &RunId,
    request_id: &str,
    host_session_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::InspectRun(InspectRunCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
        }),
    }
}

pub fn claim_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    claim_command_for_session(run_id, request_id, "session-1", Some(30_000))
}

pub fn claim_command_for_session(
    run_id: &RunId,
    request_id: &str,
    host_session_id: &str,
    requested_lease_ms: Option<u64>,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::ClaimRun(ClaimRunCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            owner_kind: RunClaimOwnerKind::InteractiveHost,
            owner_instance_id: host_session_id.to_owned(),
            mode: RunClaimMode::ExclusiveMutation,
            requested_lease_ms,
            allow_supersede: false,
            expected_current_claim_id: None,
            expected_current_claim_epoch: None,
        }),
    }
}

pub fn renew_claim_command(
    run_id: &RunId,
    claim_id: &str,
    claim_epoch: u64,
    request_id: &str,
) -> HostedRunCommand {
    renew_claim_command_for_session(run_id, claim_id, claim_epoch, request_id, "session-1")
}

pub fn renew_claim_command_for_session(
    run_id: &RunId,
    claim_id: &str,
    claim_epoch: u64,
    request_id: &str,
    host_session_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::RenewRunClaim(RenewRunClaimCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            claim_id: ClaimId(claim_id.to_owned()),
            claim_epoch,
            requested_lease_ms: Some(30_000),
        }),
    }
}

pub fn release_claim_command(
    run_id: &RunId,
    claim_id: &str,
    claim_epoch: u64,
    request_id: &str,
) -> HostedRunCommand {
    release_claim_command_for_session(run_id, claim_id, claim_epoch, request_id, "session-1")
}

pub fn release_claim_command_for_session(
    run_id: &RunId,
    claim_id: &str,
    claim_epoch: u64,
    request_id: &str,
    host_session_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::ReleaseRunClaim(ReleaseRunClaimCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            claim_id: ClaimId(claim_id.to_owned()),
            claim_epoch,
            reason: Some("transport passthrough".to_owned()),
        }),
    }
}

pub fn evidence_command(run_id: &RunId) -> HostedRunCommand {
    evidence_command_for_session(run_id, "session-1", "request-evidence")
}

pub fn evidence_command_for_session(
    run_id: &RunId,
    host_session_id: &str,
    request_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::SubmitEvidence(SubmitEvidenceCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            evidence: ais_agent_control::commands::EvidenceSubmission {
                evidence_id: "quote".to_owned(),
                kind: ais_agent_control::commands::EvidenceKind::RouteOrQuote,
                source: "quote-api".to_owned(),
                observed_at_ms: Some(1_735_000_000_000),
                chain_scope: Some("eip155:1".to_owned()),
                payload: json!({ "amount_out": "1000000" }),
                confidence: Some(0.95),
            },
            expected_version: None,
        }),
    }
}

pub fn step_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    step_command_for_session(run_id, request_id, "session-1")
}

pub fn step_command_for_session(
    run_id: &RunId,
    request_id: &str,
    host_session_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId(host_session_id.to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::StepRun(StepRunCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            until: StepUntil::CompleteOrBoundary,
            budget: Some(StepBudget {
                max_nodes: Some(8),
                max_wall_clock_ms: None,
            }),
            expected_version: None,
        }),
    }
}

pub fn signer_command(run_id: &RunId, request_id: &SignerRequestId) -> HostedRunCommand {
    signer_decision_command(
        run_id,
        request_id,
        SignerDecisionKind::Submitted,
        "request-signer",
    )
}

pub fn signer_approved_command(run_id: &RunId, request_id: &SignerRequestId) -> HostedRunCommand {
    signer_decision_command(
        run_id,
        request_id,
        SignerDecisionKind::Approved,
        "request-signer-approved",
    )
}

fn signer_decision_command(
    run_id: &RunId,
    request_id: &SignerRequestId,
    decision: SignerDecisionKind,
    host_request_id: &str,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(host_request_id.into()),
        command: RunCommand::SubmitSignerDecision(SubmitSignerDecisionCommand {
            command_id: CommandId(format!("cmd-{host_request_id}")),
            run_id: run_id.clone(),
            decision: SignerDecisionSubmission {
                request_id: request_id.clone(),
                decision,
                tx_hash: Some("0xdeadbeef".to_owned()),
                details: BTreeMap::new(),
            },
            expected_version: None,
        }),
    }
}

pub fn cancel_command(run_id: &RunId) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some("request-cancel".into()),
        command: RunCommand::CancelRun(CancelRunCommand {
            command_id: CommandId("cmd-cancel".to_owned()),
            run_id: run_id.clone(),
            reason: Some("cancelled in transport e2e".to_owned()),
            expected_version: None,
        }),
    }
}

pub fn request_cancel_command(run_id: &RunId) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some("request-cancel-pending".into()),
        command: RunCommand::RequestCancelRun(RequestCancelRunCommand {
            command_id: CommandId("cmd-cancel-pending".to_owned()),
            run_id: run_id.clone(),
            reason: Some("cancel after side effect submission".to_owned()),
            expected_version: None,
        }),
    }
}

pub fn patch_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    patch_command_with_version(
        run_id,
        request_id,
        ExpectedRuntimeVersion {
            checkpoint_seq: Some(4),
            plan_epoch: Some(2),
        },
    )
}

pub fn stale_patch_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    patch_command_with_version(
        run_id,
        request_id,
        ExpectedRuntimeVersion {
            checkpoint_seq: Some(3),
            plan_epoch: Some(1),
        },
    )
}

pub fn illegal_patch_command(run_id: &RunId, request_id: &str) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            patch: PlanPatchSubmission {
                patch_id: "patch-illegal".to_owned(),
                run_id: run_id.clone(),
                basis_checkpoint_seq: 4,
                basis_plan_epoch: 2,
                reason_code: RunFailureCode::GovernorDenied,
                target: PlanPatchTarget::NodeSet {
                    node_ids: Vec::new(),
                },
                operations: vec![PlanPatchOperation::DropBranch {
                    node_ids: Vec::new(),
                }],
                expected_outcome: None,
            },
            expected_version: Some(ExpectedRuntimeVersion {
                checkpoint_seq: Some(4),
                plan_epoch: Some(2),
            }),
        }),
    }
}

pub fn envelope_command(run_id: &RunId, envelope_id: &str, request_id: &str) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::SubmitEnvelope(SubmitEnvelopeCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            envelope: EnvelopeSubmission {
                envelope_id: envelope_id.to_owned(),
                kind: EnvelopeKind::EvmEnvelope,
                chain: "eip155:1".to_owned(),
                payload: json!({"raw_tx":"0x0102"}),
                expected_effect: Some(json!({
                    "effect_id":"effect.swap",
                    "kind":"state_transition",
                    "assertions":[],
                    "tolerance_hint":null
                })),
                provenance: Some("transport.e2e".to_owned()),
            },
            expected_version: None,
        }),
    }
}

fn patch_command_with_version(
    run_id: &RunId,
    request_id: &str,
    expected_version: ExpectedRuntimeVersion,
) -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(request_id.into()),
        command: RunCommand::SubmitPlanPatch(SubmitPlanPatchCommand {
            command_id: CommandId(format!("cmd-{request_id}")),
            run_id: run_id.clone(),
            patch: patch_submission(run_id),
            expected_version: Some(expected_version),
        }),
    }
}

fn patch_submission(run_id: &RunId) -> PlanPatchSubmission {
    PlanPatchSubmission {
        patch_id: "patch-transport-1".to_owned(),
        run_id: run_id.clone(),
        basis_checkpoint_seq: 4,
        basis_plan_epoch: 2,
        reason_code: RunFailureCode::GovernorDenied,
        target: PlanPatchTarget::FailedFragment {
            node_ids: vec!["derive-failed".to_owned()],
        },
        operations: vec![PlanPatchOperation::ReplaceFragment {
            fragment: json!({
                "roots": ["derive-recovered"],
                "terminals": ["derive-recovered"],
                "nodes": {
                    "derive-recovered": {
                        "node_id": "derive-recovered",
                        "kind": "derive",
                        "origin": "driver_fragment",
                        "status": "pending",
                        "depends_on": [],
                        "inputs": [],
                        "evidence_refs": [],
                        "payload": {
                            "type": "derive",
                            "derive_kind": "parameter",
                            "derivation_hint": "recover derive",
                            "output_key": "quote"
                        },
                        "implementation_hint": null,
                        "expected_effect_ref": null
                    }
                },
                "live_binding_hints": {}
            }),
            preserved_effect_refs: Vec::new(),
        }],
        expected_outcome: None,
    }
}
