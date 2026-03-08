use super::*;
use ais_engine::{EngineEvent, EngineEventRecord, EngineEventType, EngineRunnerState};
use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider};
use ais_sdk::PlanDocument;
use serde_json::json;
use serde_json::Map;

fn need_user_confirm_summary_with_bundle(node_id: &str, bundle: &[&str]) -> PauseSummary {
    let items = bundle
        .iter()
        .map(|node_id| super::super::summary::NeedUserConfirmBundleItem {
            node_id: (*node_id).to_string(),
            action_ref: None,
            chain: None,
            execution_type: None,
            risk_level: Some(3),
            params: Vec::new(),
            confirmation_hash: None,
        })
        .collect::<Vec<_>>();
    PauseSummary {
        raw_reason: Some(format!("need_user_confirm:{node_id}")),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some(node_id.to_string()),
        need_user_confirm: Some(super::super::summary::NeedUserConfirmSummary {
            reason_code: Some("threshold_risk_level_exceeded".to_string()),
            reason: Some("policy".to_string()),
            confirmation_hash: Some("abc".to_string()),
            confirmation_summary: Some(json!({"risk_level": 2})),
            confirmation_bundle: Some(super::super::summary::ConfirmationBundle {
                bundle_id: format!("bundle:{}", bundle.join(",")),
                segment_id: "seg_3".to_string(),
                current_node_id: node_id.to_string(),
                items,
            }),
        }),
        last_error_reason: None,
    }
}

#[test]
fn assist_threshold_matches_confirmation_summary_risk_level() {
    let summary = need_user_confirm_summary_with_bundle("swap-1", &[]);
    assert!(should_attempt_assist_auto_approve(&summary, 2));
    assert!(!should_attempt_assist_auto_approve(&summary, 1));
}

#[test]
fn decision_path_is_enumerable() {
    let summary = need_user_confirm_summary_with_bundle("swap-1", &[]);

    let yolo_policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Yolo, None, None);
    assert_eq!(
        yolo_policy.classify_path(&summary),
        DecisionPath::YoloAutoApprove
    );

    let assist_policy = AgentDecisionPolicy::new(
        ApprovalsMode::Assist,
        Some(2),
        Some(LlmBrain::new(ScriptedLlmProvider::from_responses(vec![]))),
    );
    assert_eq!(
        assist_policy.classify_path(&summary),
        DecisionPath::AssistLlmAutoApprove
    );

    let safe_policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    assert_eq!(
        safe_policy.classify_path(&summary),
        DecisionPath::ManualPrompt
    );
}

#[test]
fn assist_policy_uses_llm_for_low_risk_confirm() {
    let mut policy = AgentDecisionPolicy::new(
        ApprovalsMode::Assist,
        Some(2),
        Some(LlmBrain::new(ScriptedLlmProvider::from_responses(vec![
            Ok(CompleteWithToolsResponse {
                assistant_content: Some("approve".to_string()),
                tool_calls: vec![ToolCall {
                    id: "tool-1".to_string(),
                    name: "confirm".to_string(),
                    arguments: json!({"decision":"approve"}),
                }],
            }),
        ]))),
    );
    let mut builder = CommandBuilder::new("run-test");
    let summary = need_user_confirm_summary_with_bundle("swap-1", &[]);

    let commands = policy
        .decide(&summary, &mut builder)
        .expect("assist llm should return command");
    assert_eq!(commands.len(), 1);
    let command = commands[0].command.clone();
    assert_eq!(
        command.command_type,
        ais_engine::EngineCommandType::UserConfirm
    );
    assert_eq!(
        command
            .data
            .get("decision")
            .and_then(serde_json::Value::as_str),
        Some("approve")
    );
}

#[test]
fn assist_threshold_outside_range_falls_back_to_manual_path() {
    let mut summary = need_user_confirm_summary_with_bundle("transfer-1", &[]);
    summary
        .need_user_confirm
        .as_mut()
        .unwrap()
        .confirmation_summary = Some(json!({"risk_level": 4}));
    let policy = AgentDecisionPolicy::new(
        ApprovalsMode::Assist,
        Some(2),
        Some(LlmBrain::new(ScriptedLlmProvider::from_responses(vec![]))),
    );
    assert_eq!(policy.classify_path(&summary), DecisionPath::ManualPrompt);
}

#[test]
fn llm_brain_supports_detail_lookup_then_confirm() {
    let provider = ScriptedLlmProvider::from_responses(vec![
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("need detail".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-1".to_string(),
                name: "get_candidate_detail".to_string(),
                arguments: json!({"refs":["p@1/swap"]}),
            }],
        }),
        Ok(CompleteWithToolsResponse {
            assistant_content: Some("approve".to_string()),
            tool_calls: vec![ToolCall {
                id: "tool-2".to_string(),
                name: "confirm".to_string(),
                arguments: json!({"decision":"approve"}),
            }],
        }),
    ]);
    let mut context = CandidateContext::default();
    context.detail_by_ref.insert(
        "p@1/swap".to_string(),
        json!({"ref":"p@1/swap","kind":"action"}),
    );
    let mut brain = LlmBrain::new(provider).with_candidate_context(context);
    let mut builder = CommandBuilder::new("run-test");
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:swap-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("swap-1".to_string()),
        need_user_confirm: None,
        last_error_reason: None,
    };

    let commands = brain
        .decide(&summary, &mut builder)
        .expect("llm must eventually return engine command");
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].command.command_type,
        ais_engine::EngineCommandType::UserConfirm
    );
}

#[test]
fn manual_always_approve_short_circuits_prompt() {
    let mut policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    policy.manual_always_approve_this_run = true;
    let mut builder = CommandBuilder::new("run-test");
    let summary = need_user_confirm_summary_with_bundle("transfer-1", &[]);

    let commands = policy
        .decide(&summary, &mut builder)
        .expect("must auto-approve");
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].command.command_type,
        ais_engine::EngineCommandType::UserConfirm
    );
    assert_eq!(
        commands[0]
            .command
            .data
            .get("decision")
            .and_then(serde_json::Value::as_str),
        Some("approve")
    );
}

#[test]
fn manual_bundle_approve_short_circuits_followup_confirms() {
    let mut policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    policy.manual_bundle_approve = Some(PendingBundleApproval {
        bundle_id: "bundle:seg_3__a_erc20_transfer".to_string(),
        node_ids: std::iter::once("seg_3__a_erc20_transfer".to_string()).collect(),
    });
    let mut builder = CommandBuilder::new("run-test");
    let summary = need_user_confirm_summary_with_bundle(
        "seg_3__a_erc20_transfer",
        &["seg_3__a_erc20_transfer"],
    );

    let commands = policy
        .decide(&summary, &mut builder)
        .expect("must approve current bundle node");
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].command.command_type,
        ais_engine::EngineCommandType::UserConfirm
    );
    assert_eq!(
        commands[0]
            .command
            .data
            .get("decision")
            .and_then(serde_json::Value::as_str),
        Some("approve")
    );
    assert!(policy.manual_bundle_approve.is_none());
}

#[test]
fn manual_bundle_approve_does_not_apply_to_nodes_outside_current_bundle() {
    let mut policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    policy.manual_bundle_approve = Some(PendingBundleApproval {
        bundle_id: "bundle:seg_3__a_erc20_transfer".to_string(),
        node_ids: std::iter::once("seg_3__a_erc20_transfer".to_string()).collect(),
    });
    let mut builder = CommandBuilder::new("run-test");
    let summary = need_user_confirm_summary_with_bundle(
        "seg_3__a_native_transfer",
        &["seg_3__a_native_transfer"],
    );

    let decision = policy.try_apply_manual_bundle_approve(&summary, &mut builder);
    assert!(decision.is_none());
    assert!(policy.manual_bundle_approve.is_none());
}

#[test]
fn help_contract_describes_bundle_scoped_approve_all() {
    let contract = need_user_confirm_command_contract();
    assert_eq!(contract.approve_current, "approve current action");
    assert_eq!(
        contract.approve_all_bundle,
        "use `approve_all` once to approve all actions shown in this bundle"
    );
}

#[test]
fn manual_bundle_approve_survives_current_node_advancement_with_production_bundle_id() {
    let plan = PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: None,
        nodes: vec![
            json!({
                "id":"seg_3__a_native_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"5"},
                    "to":{"ref":"inputs.recipient"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"evm-native-utils@0.0.1/native-transfer"}
                }
            }),
            json!({
                "id":"seg_3__a_token_transfer",
                "kind":"action_ref",
                "chain":"eip155:31338",
                "execution":{"type":"evm_call"},
                "bindings":{"params":{
                    "amount":{"lit":"10"},
                    "to":{"ref":"inputs.recipient"},
                    "token":{"ref":"inputs.token.address"}
                }},
                "extensions":{
                    "risk_level":3,
                    "plan_sketch":{"candidate_ref":"erc20@0.0.2/transfer"}
                }
            }),
        ],
        extensions: Map::new(),
    };
    let state = EngineRunnerState {
        runtime: json!({
            "inputs": {
                "recipient": "0xrecipient",
                "token": {
                    "address": "0xtoken"
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let first_summary = super::super::summary::summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_native_transfer"),
        &[{
            let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
            event.node_id = Some("seg_3__a_native_transfer".to_string());
            event.data = serde_json::Map::from_iter([
                (
                    "reason_code".to_string(),
                    json!("threshold_risk_level_exceeded"),
                ),
                ("reason".to_string(), json!("manual review required")),
                (
                    "details".to_string(),
                    json!({
                        "confirmation_hash":"0xnative",
                        "confirmation_summary":{
                            "node_id":"seg_3__a_native_transfer",
                            "chain":"eip155:31338",
                            "action_ref":"action:evm-native-utils@0.0.1/native-transfer",
                            "execution_type":"evm_call",
                            "risk_level":3
                        }
                    }),
                ),
            ]);
            EngineEventRecord::new("run-test", 1, "1970-01-01T00:00:00Z", event)
        }],
        Some(&plan),
        Some(&state),
    );
    let first_bundle = first_summary
        .need_user_confirm
        .as_ref()
        .and_then(|need| need.confirmation_bundle.as_ref())
        .expect("first bundle");

    let mut policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    policy.manual_bundle_approve = Some(PendingBundleApproval {
        bundle_id: first_bundle.bundle_id.clone(),
        node_ids: std::iter::once("seg_3__a_token_transfer".to_string()).collect(),
    });

    let followup_summary = super::super::summary::summarize_pause_with_context(
        Some("need_user_confirm:seg_3__a_token_transfer"),
        &[{
            let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
            event.node_id = Some("seg_3__a_token_transfer".to_string());
            event.data = serde_json::Map::from_iter([
                (
                    "reason_code".to_string(),
                    json!("threshold_risk_level_exceeded"),
                ),
                ("reason".to_string(), json!("manual review required")),
                (
                    "details".to_string(),
                    json!({
                        "confirmation_hash":"0xtoken",
                        "confirmation_summary":{
                            "node_id":"seg_3__a_token_transfer",
                            "chain":"eip155:31338",
                            "action_ref":"action:erc20@0.0.2/transfer",
                            "execution_type":"evm_call",
                            "risk_level":3
                        }
                    }),
                ),
            ]);
            EngineEventRecord::new("run-test", 2, "1970-01-01T00:00:01Z", event)
        }],
        Some(&plan),
        Some(&state),
    );

    let mut builder = CommandBuilder::new("run-test");
    let commands = policy
        .decide(&followup_summary, &mut builder)
        .expect("must approve next node in same bundle");

    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].command.command_type,
        ais_engine::EngineCommandType::UserConfirm
    );
    assert_eq!(
        commands[0]
            .command
            .data
            .get("decision")
            .and_then(serde_json::Value::as_str),
        Some("approve")
    );
    assert!(policy.manual_bundle_approve.is_none());
}
