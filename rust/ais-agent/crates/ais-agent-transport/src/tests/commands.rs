use std::collections::BTreeMap;

use serde_json::json;

use ais_agent_control::{
    commands::{
        BeginRunCommand, CancelRunCommand, ClaimRunCommand, EnvelopeKind, EnvelopeSubmission,
        ExpectedRuntimeVersion, InspectRunCommand, MissionSubmission, ReleaseRunClaimCommand,
        RenewRunClaimCommand, RequestCancelRunCommand, RunCommand, SignerResolutionKind,
        SignerResolutionSubmission, StepBudget, StepRunCommand, StepUntil, SubmitEnvelopeCommand,
        SubmitEvidenceCommand, SubmitPlanPatchCommand, SubmitSignerResolutionCommand,
    },
    execution_artifact::{
        BranchStage, BranchTarget, ComparisonOperator, ContinuationStage, EvmTransactionCandidate,
        ExecutionArtifactLaunchSpec, ExecutionChainFamily, ExecutionStage,
        ExecutionTransactionCandidate, OutputExportSpec, PredicateSpec, TransactionStage, ValueRef,
    },
    ids::{ClaimId, CommandId, IdempotencyKey, RunId, SignerRequestId},
    launch_spec::{LaunchSpecSubmission, PrebuiltFragmentLaunchSpec, ReflectionRequestLaunchSpec},
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
            launch_spec: Some(LaunchSpecSubmission::PrebuiltFragment(
                PrebuiltFragmentLaunchSpec::default(),
            )),
        }),
    }
}

pub fn sample_reflection_begin_command() -> HostedRunCommand {
    let mut command = sample_begin_command();
    let RunCommand::BeginRun(begin) = &mut command.command else {
        panic!("sample_begin_command must produce begin_run");
    };
    begin.launch_spec = Some(LaunchSpecSubmission::ReflectionRequest(
        ReflectionRequestLaunchSpec {
            request: json!({
                "protocol_package_id": "owliabot.uniswap_v3",
                "action_key": "uniswap_v3_swap"
            }),
        },
    ));
    command
}

pub fn sample_execution_artifact_begin_command() -> HostedRunCommand {
    let mut command = sample_begin_command();
    let RunCommand::BeginRun(begin) = &mut command.command else {
        panic!("sample_begin_command must produce begin_run");
    };
    begin.launch_spec = Some(LaunchSpecSubmission::ExecutionArtifact(
        ExecutionArtifactLaunchSpec {
            protocol_package_id: "owliabot.uniswap_v3".to_owned(),
            action_key: "uniswap_v3_swap".to_owned(),
            chain_family: ExecutionChainFamily::Evm,
            allowed_chains: vec!["8453".to_owned()],
            entry_stage_id: "stage.allowance".into(),
            actor: None,
            transactions: vec![
                ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                    candidate_id: "swap.direct".into(),
                    to: "0x1111111111111111111111111111111111111111".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0xdeadbeef".to_owned()),
                }),
                ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                    candidate_id: "swap.approval".into(),
                    to: "0x3333333333333333333333333333333333333333".to_owned(),
                    value: Some("0".to_owned()),
                    calldata: Some("0x095ea7b3".to_owned()),
                }),
            ],
            stages: vec![
                ExecutionStage::Branch(BranchStage {
                    stage_id: "stage.allowance".into(),
                    predicate: PredicateSpec::Comparison {
                        left: ValueRef::Ref {
                            reference: "refs.allowance.current_atomic".to_owned(),
                        },
                        op: ComparisonOperator::Lt,
                        right: ValueRef::Cel {
                            expression: "mul_div(refs.swap.amount_in_atomic, 1, 1)".to_owned(),
                        },
                    },
                    if_true: BranchTarget::GotoStage {
                        stage_id: "stage.approval".into(),
                    },
                    if_false: BranchTarget::GotoStage {
                        stage_id: "stage.swap".into(),
                    },
                }),
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.approval".into(),
                    candidate_ref: "swap.approval".into(),
                    exports: Vec::new(),
                    next_stage_id: Some("stage.swap".into()),
                }),
                ExecutionStage::Transaction(TransactionStage {
                    stage_id: "stage.swap".into(),
                    candidate_ref: "swap.direct".into(),
                    exports: vec![OutputExportSpec {
                        output_key: "swap.received_atomic".into(),
                        source: ValueRef::Ref {
                            reference: "refs.post.swap.received_atomic".to_owned(),
                        },
                    }],
                    next_stage_id: Some("stage.continue".into()),
                }),
                ExecutionStage::Continuation(ContinuationStage {
                    stage_id: "stage.continue".into(),
                    required_outputs: vec!["swap.received_atomic".into()],
                    package_entry: "build_aave_supply_from_swap_output".into(),
                    next_stage_id: None,
                }),
            ],
            observations: Vec::new(),
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            expected_effects: Vec::new(),
            execution_policy: None,
            risk_class: Some("bounded_swap".to_owned()),
            risk_tags: vec!["router_call".to_owned(), "transport_test".to_owned()],
            decoded_intent: Some(json!({
                "kind": "bounded_swap",
                "token_in": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "token_out": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "recipient": "0xcccccccccccccccccccccccccccccccccccccccc",
                "amount_in_exact": "1000",
                "amount_out_min": "900"
            })),
            candidate_envelopes: vec![json!({
                "candidate_ref": "swap.direct",
                "candidate_kind": "evm_transaction",
                "risk_class": "bounded_swap",
                "source": {
                    "kind": "http_api",
                    "source_id": "transport-test"
                },
                "intent": {
                    "kind": "bounded_swap",
                    "token_in": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "token_out": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "recipient": "0xcccccccccccccccccccccccccccccccccccccccc",
                    "amount_in_exact": "1000",
                    "amount_out_min": "900"
                },
                "validation_plan": {
                    "static_constraints": [{
                        "chain": "8453",
                        "target_allowlist": ["0x1111111111111111111111111111111111111111"],
                        "selector_allowlist": ["0xdeadbeef"]
                    }],
                    "failure_mode": "reject"
                }
            })],
            decode_spec: Some(json!({
                "target_allowlist": ["0x1111111111111111111111111111111111111111"],
                "entrypoints": [{
                    "selector": "0xdeadbeef",
                    "abi_fragment": "function exactInputSingle(bytes data)",
                    "intent_kind": "bounded_swap"
                }],
                "fallback_mode": "reject"
            })),
            validation_plan: Some(json!({
                "static_constraints": [{
                    "chain": "8453",
                    "target_allowlist": ["0x1111111111111111111111111111111111111111"],
                    "selector_allowlist": ["0xdeadbeef"]
                }],
                "require_simulation": true,
                "failure_mode": "reject"
            })),
            evidence: json!({
                "quote": {
                    "quotedAtMs": 1710000000000u64
                }
            }),
            metadata: BTreeMap::from([("source".to_owned(), json!("transport-test"))]),
        },
    ));
    command
}

pub fn sample_owliabot_uniswap_swap_begin_command() -> HostedRunCommand {
    HostedRunCommand {
        host_session_id: HostSessionId("test-session".to_owned()),
        host_request_id: Some("test-session:begin".into()),
        command: RunCommand::BeginRun(BeginRunCommand {
            command_id: CommandId("cmd-test-session:begin".to_owned()),
            idempotency_key: IdempotencyKey("test-session:begin".to_owned()),
            mission: MissionSubmission {
                goal: "owliabot:owliabot.uniswap_v3:uniswap_v3_swap".to_owned(),
                allowed_chains: vec!["8453".to_owned()],
                constraints: BTreeMap::from([
                    ("owliabot_action_key".to_owned(), json!("uniswap_v3_swap")),
                    (
                        "owliabot_protocol_package_id".to_owned(),
                        json!("owliabot.uniswap_v3"),
                    ),
                    ("owliabot_execution_mode".to_owned(), json!("harness")),
                    ("owliabot_chain_family".to_owned(), json!("evm")),
                    (
                        "owliabot_execution_artifact".to_owned(),
                        json!({
                            "protocol_package_id": "owliabot.uniswap_v3",
                            "action_key": "uniswap_v3_swap",
                            "chain_family": "evm",
                            "allowed_chains": ["8453"],
                            "entry_stage_id": "stage.allowance",
                            "transactions": [
                                {
                                    "kind": "evm_transaction",
                                    "candidate_id": "swap.direct",
                                    "to": "0x1111111111111111111111111111111111111111",
                                    "value": "0",
                                    "calldata": "0xdeadbeef"
                                },
                                {
                                    "kind": "evm_transaction",
                                    "candidate_id": "swap.approval",
                                    "to": "0x3333333333333333333333333333333333333333",
                                    "value": "0",
                                    "calldata": "0x095ea7b3"
                                }
                            ],
                            "stages": [
                                {
                                    "kind": "branch",
                                    "stage_id": "stage.allowance",
                                    "predicate": {
                                        "kind": "comparison",
                                        "left": {
                                            "kind": "ref",
                                            "ref": "refs.allowance.current_atomic"
                                        },
                                        "op": "lt",
                                        "right": {
                                            "kind": "cel",
                                            "expression": "mul_div(refs.swap.amount_in_atomic, 1, 1)"
                                        }
                                    },
                                    "if_true": {
                                        "kind": "goto_stage",
                                        "stage_id": "stage.approval"
                                    },
                                    "if_false": {
                                        "kind": "goto_stage",
                                        "stage_id": "stage.swap"
                                    }
                                },
                                {
                                    "kind": "transaction",
                                    "stage_id": "stage.approval",
                                    "candidate_ref": "swap.approval",
                                    "exports": [],
                                    "next_stage_id": "stage.swap"
                                },
                                {
                                    "kind": "transaction",
                                    "stage_id": "stage.swap",
                                    "candidate_ref": "swap.direct",
                                    "exports": [
                                        {
                                            "output_key": "swap.received_atomic",
                                            "source": {
                                                "kind": "ref",
                                                "ref": "refs.post.swap.received_atomic"
                                            }
                                        }
                                    ],
                                    "next_stage_id": "stage.continue"
                                },
                                {
                                    "kind": "continuation",
                                    "stage_id": "stage.continue",
                                    "required_outputs": ["swap.received_atomic"],
                                    "package_entry": "build_aave_supply_from_swap_output"
                                }
                            ],
                            "preconditions": [],
                            "postconditions": [],
                            "expected_effects": [],
                            "evidence": {
                                "quote": {
                                    "quoted_at_ms": 1710000000000u64
                                }
                            },
                            "metadata": {
                                "source": "skill:uniswap-v3-swap",
                                "tool_name": "ais_run_harness"
                            }
                        }),
                    ),
                ]),
                budget: None,
                metadata: BTreeMap::from([
                    ("owliabot_agent_id".to_owned(), json!("test-agent")),
                    ("source".to_owned(), json!("skill:uniswap-v3-swap")),
                    ("tool_name".to_owned(), json!("ais_run_harness")),
                ]),
            },
            launch_spec: Some(LaunchSpecSubmission::ExecutionArtifact(
                ExecutionArtifactLaunchSpec {
                    protocol_package_id: "owliabot.uniswap_v3".to_owned(),
                    action_key: "uniswap_v3_swap".to_owned(),
                    chain_family: ExecutionChainFamily::Evm,
                    allowed_chains: vec!["8453".to_owned()],
                    entry_stage_id: "stage.allowance".into(),
                    actor: None,
                    transactions: vec![
                        ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                            candidate_id: "swap.direct".into(),
                            to: "0x1111111111111111111111111111111111111111".to_owned(),
                            value: Some("0".to_owned()),
                            calldata: Some("0xdeadbeef".to_owned()),
                        }),
                        ExecutionTransactionCandidate::EvmTransaction(EvmTransactionCandidate {
                            candidate_id: "swap.approval".into(),
                            to: "0x3333333333333333333333333333333333333333".to_owned(),
                            value: Some("0".to_owned()),
                            calldata: Some("0x095ea7b3".to_owned()),
                        }),
                    ],
                    stages: vec![
                        ExecutionStage::Branch(BranchStage {
                            stage_id: "stage.allowance".into(),
                            predicate: PredicateSpec::Comparison {
                                left: ValueRef::Ref {
                                    reference: "refs.allowance.current_atomic".to_owned(),
                                },
                                op: ComparisonOperator::Lt,
                                right: ValueRef::Cel {
                                    expression: "mul_div(refs.swap.amount_in_atomic, 1, 1)"
                                        .to_owned(),
                                },
                            },
                            if_true: BranchTarget::GotoStage {
                                stage_id: "stage.approval".into(),
                            },
                            if_false: BranchTarget::GotoStage {
                                stage_id: "stage.swap".into(),
                            },
                        }),
                        ExecutionStage::Transaction(TransactionStage {
                            stage_id: "stage.approval".into(),
                            candidate_ref: "swap.approval".into(),
                            exports: Vec::new(),
                            next_stage_id: Some("stage.swap".into()),
                        }),
                        ExecutionStage::Transaction(TransactionStage {
                            stage_id: "stage.swap".into(),
                            candidate_ref: "swap.direct".into(),
                            exports: vec![OutputExportSpec {
                                output_key: "swap.received_atomic".into(),
                                source: ValueRef::Ref {
                                    reference: "refs.post.swap.received_atomic".to_owned(),
                                },
                            }],
                            next_stage_id: Some("stage.continue".into()),
                        }),
                        ExecutionStage::Continuation(ContinuationStage {
                            stage_id: "stage.continue".into(),
                            required_outputs: vec!["swap.received_atomic".into()],
                            package_entry: "build_aave_supply_from_swap_output".into(),
                            next_stage_id: None,
                        }),
                    ],
                    observations: Vec::new(),
                    preconditions: Vec::new(),
                    postconditions: Vec::new(),
                    expected_effects: Vec::new(),
                    execution_policy: None,
                    risk_class: None,
                    risk_tags: Vec::new(),
                    decoded_intent: None,
                    candidate_envelopes: Vec::new(),
                    decode_spec: None,
                    validation_plan: None,
                    evidence: json!({
                        "quote": {
                            "quotedAtMs": 1710000000000u64
                        }
                    }),
                    metadata: BTreeMap::from([
                        ("source".to_owned(), json!("skill:uniswap-v3-swap")),
                        ("tool_name".to_owned(), json!("ais_run_harness")),
                    ]),
                },
            )),
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
        SignerResolutionKind::Submitted,
        "request-signer",
    )
}

fn signer_decision_command(
    run_id: &RunId,
    request_id: &SignerRequestId,
    decision: SignerResolutionKind,
    host_request_id: &str,
) -> HostedRunCommand {
    let signed_payload = matches!(decision, SignerResolutionKind::Signed)
        .then(|| serde_json::json!({"raw_tx":"0x0102"}));
    HostedRunCommand {
        host_session_id: HostSessionId("session-1".to_owned()),
        host_request_id: Some(host_request_id.into()),
        command: RunCommand::SubmitSignerResolution(SubmitSignerResolutionCommand {
            command_id: CommandId(format!("cmd-{host_request_id}")),
            run_id: run_id.clone(),
            resolution: SignerResolutionSubmission {
                request_id: request_id.clone(),
                kind: decision,
                tx_hash: Some("0xdeadbeef".to_owned()),
                signed_payload,
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

#[test]
fn hosted_command_round_trips_reflection_launch_spec() {
    let encoded =
        serde_json::to_string(&sample_reflection_begin_command()).expect("encode command");
    assert!(encoded.contains("\"kind\":\"reflection_request\""));

    let decoded: HostedRunCommand = serde_json::from_str(&encoded).expect("decode command");
    let RunCommand::BeginRun(begin) = decoded.command else {
        panic!("expected begin_run");
    };

    match begin.launch_spec.expect("launch_spec") {
        LaunchSpecSubmission::ReflectionRequest(spec) => {
            assert_eq!(
                spec.request["protocol_package_id"],
                json!("owliabot.uniswap_v3")
            );
        }
        other => panic!("unexpected launch_spec: {other:?}"),
    }
}

#[test]
fn hosted_command_round_trips_execution_artifact_launch_spec() {
    let encoded =
        serde_json::to_string(&sample_execution_artifact_begin_command()).expect("encode command");
    assert!(encoded.contains("\"kind\":\"execution_artifact\""));

    let decoded: HostedRunCommand = serde_json::from_str(&encoded).expect("decode command");
    let RunCommand::BeginRun(begin) = decoded.command else {
        panic!("expected begin_run");
    };

    match begin.launch_spec.expect("launch_spec") {
        LaunchSpecSubmission::ExecutionArtifact(spec) => {
            assert_eq!(spec.protocol_package_id, "owliabot.uniswap_v3");
            assert_eq!(spec.action_key, "uniswap_v3_swap");
            assert_eq!(spec.allowed_chains, vec!["8453".to_owned()]);
            assert_eq!(spec.transactions.len(), 2);
            assert_eq!(spec.stages.len(), 4);
            assert_eq!(spec.risk_class.as_deref(), Some("bounded_swap"));
            assert_eq!(
                spec.risk_tags,
                vec!["router_call".to_owned(), "transport_test".to_owned()]
            );
            assert_eq!(spec.candidate_envelopes.len(), 1);
            assert_eq!(
                spec.candidate_envelopes[0]["candidate_kind"],
                json!("evm_transaction")
            );
            assert_eq!(
                spec.decode_spec
                    .as_ref()
                    .and_then(|spec| spec.get("fallback_mode")),
                Some(&json!("reject"))
            );
            assert_eq!(
                spec.validation_plan
                    .as_ref()
                    .and_then(|plan| plan.get("require_simulation")),
                Some(&json!(true))
            );
            let export_stage = spec.stage("stage.swap").expect("swap stage");
            let tx_stage = export_stage.as_transaction().expect("transaction stage");
            assert_eq!(
                tx_stage.exports[0].output_key.as_str(),
                "swap.received_atomic"
            );
            assert_eq!(spec.metadata.get("source"), Some(&json!("transport-test")));
        }
        other => panic!("unexpected launch_spec: {other:?}"),
    }
}

#[test]
fn hosted_command_round_trips_owliabot_uniswap_skill_boundary_envelope() {
    let encoded = serde_json::to_string(&sample_owliabot_uniswap_swap_begin_command())
        .expect("encode owliabot uniswap begin command");
    assert!(encoded.contains("owliabot:owliabot.uniswap_v3:uniswap_v3_swap"));
    assert!(encoded.contains("\"kind\":\"execution_artifact\""));

    let decoded: HostedRunCommand = serde_json::from_str(&encoded).expect("decode command");
    let RunCommand::BeginRun(begin) = decoded.command else {
        panic!("expected begin_run");
    };

    assert_eq!(
        begin.mission.goal,
        "owliabot:owliabot.uniswap_v3:uniswap_v3_swap"
    );
    assert_eq!(begin.mission.allowed_chains, vec!["8453".to_owned()]);
    assert_eq!(
        begin
            .mission
            .constraints
            .get("owliabot_protocol_package_id"),
        Some(&json!("owliabot.uniswap_v3"))
    );
    assert_eq!(
        begin.mission.metadata.get("tool_name"),
        Some(&json!("ais_run_harness"))
    );

    match begin.launch_spec.expect("launch_spec") {
        LaunchSpecSubmission::ExecutionArtifact(spec) => {
            assert_eq!(spec.protocol_package_id, "owliabot.uniswap_v3");
            assert_eq!(spec.action_key, "uniswap_v3_swap");
            assert_eq!(spec.allowed_chains, vec!["8453".to_owned()]);
            assert_eq!(spec.transactions.len(), 2);
            assert_eq!(spec.stages.len(), 4);
            assert_eq!(
                spec.metadata.get("source"),
                Some(&json!("skill:uniswap-v3-swap"))
            );
        }
        other => panic!("unexpected launch_spec: {other:?}"),
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
