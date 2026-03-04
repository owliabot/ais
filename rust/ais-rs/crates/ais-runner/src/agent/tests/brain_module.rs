use super::*;
use ais_llm::{CompleteWithToolsResponse, ScriptedLlmProvider};
use serde_json::json;

#[test]
fn assist_threshold_matches_confirmation_summary_risk_level() {
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:swap-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("swap-1".to_string()),
        need_user_confirm: Some(super::super::summary::NeedUserConfirmSummary {
            reason_code: Some("threshold_risk_level_exceeded".to_string()),
            reason: Some("policy".to_string()),
            confirmation_hash: Some("abc".to_string()),
            confirmation_summary: Some(json!({"risk_level": 2})),
            segment_bundle: Vec::new(),
        }),
        last_error_reason: None,
    };
    assert!(should_attempt_assist_auto_approve(&summary, 2));
    assert!(!should_attempt_assist_auto_approve(&summary, 1));
}

#[test]
fn decision_path_is_enumerable() {
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:swap-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("swap-1".to_string()),
        need_user_confirm: Some(super::super::summary::NeedUserConfirmSummary {
            reason_code: Some("threshold_risk_level_exceeded".to_string()),
            reason: Some("policy".to_string()),
            confirmation_hash: Some("abc".to_string()),
            confirmation_summary: Some(json!({"risk_level": 2})),
            segment_bundle: Vec::new(),
        }),
        last_error_reason: None,
    };

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
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:swap-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("swap-1".to_string()),
        need_user_confirm: Some(super::super::summary::NeedUserConfirmSummary {
            reason_code: Some("threshold_risk_level_exceeded".to_string()),
            reason: Some("policy".to_string()),
            confirmation_hash: Some("abc".to_string()),
            confirmation_summary: Some(json!({"risk_level": 2})),
            segment_bundle: Vec::new(),
        }),
        last_error_reason: None,
    };

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
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:transfer-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("transfer-1".to_string()),
        need_user_confirm: Some(super::super::summary::NeedUserConfirmSummary {
            reason_code: Some("threshold_risk_level_exceeded".to_string()),
            reason: Some("policy".to_string()),
            confirmation_hash: Some("abc".to_string()),
            confirmation_summary: Some(json!({"risk_level": 4})),
            segment_bundle: Vec::new(),
        }),
        last_error_reason: None,
    };
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
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:transfer-1".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("transfer-1".to_string()),
        need_user_confirm: None,
        last_error_reason: None,
    };

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
fn manual_segment_batch_approve_short_circuits_followup_confirms() {
    let mut policy =
        AgentDecisionPolicy::<ScriptedLlmProvider>::new(ApprovalsMode::Safe, None, None);
    policy.manual_batch_approve_segment = Some("seg_3".to_string());
    let mut builder = CommandBuilder::new("run-test");
    let summary = PauseSummary {
        raw_reason: Some("need_user_confirm:seg_3__a_erc20_transfer".to_string()),
        kind: PauseKind::NeedUserConfirm,
        node_id: Some("seg_3__a_erc20_transfer".to_string()),
        need_user_confirm: None,
        last_error_reason: None,
    };

    let commands = policy
        .decide(&summary, &mut builder)
        .expect("must batch approve same segment");
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
