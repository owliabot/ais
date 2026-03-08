#[test]
fn load_or_init_state_rejects_legacy_checkpoint_node_ids() {
    let checkpoint_path = write_temp_file("legacy-checkpoint", "");
    let checkpoint = create_checkpoint_document(
        "run-legacy",
        "legacy-plan-hash",
        CheckpointEngineState {
            completed_node_ids: vec!["seg_1/q1".to_string()],
            ..CheckpointEngineState::default()
        },
        Some(json!({})),
        Some(json!({
            "schema":"ais-plan/0.0.3",
            "nodes":[{"id":"seg_1/q1"}]
        })),
        None,
    );
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };

    let error = super::load_or_init_state(&command, "current-plan-hash", json!({}))
        .expect_err("legacy checkpoint must be rejected");
    assert!(matches!(error, RunnerError::CheckpointLoad { .. }));
    assert!(error
        .to_string()
        .contains("legacy checkpoint is not supported"));
}

#[test]
fn load_or_init_state_dedupes_checkpoint_plan_snapshot_nodes() {
    let checkpoint_path = write_temp_file("dedupe-checkpoint", "");
    let checkpoint = create_checkpoint_document(
        "run-dedupe",
        "checkpoint-plan-hash",
        CheckpointEngineState::default(),
        Some(json!({})),
        Some(json!({
            "schema":"ais-plan/0.0.3",
            "nodes":[
                {"id":"seg_1__q1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q1.outputs","mode":"set"}]},
                {"id":"seg_1__q1","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q1.outputs","mode":"set"}]},
                {"id":"seg_1__q2","chain":"eip155:1","execution":{"type":"evm_read"},"writes":[{"path":"nodes.seg_1__q2.outputs","mode":"set"}]}
            ]
        })),
        None,
    );
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };

    let (
        state,
        resumed,
        checkpoint_plan,
        checkpoint_hash,
        _checkpoint_ledger,
        checkpoint_extensions,
        pending_resume_traces,
    ) = super::load_or_init_state(&command, "current-plan-hash", json!({})).expect("load state");
    assert!(resumed);
    assert_eq!(checkpoint_hash.as_deref(), Some("checkpoint-plan-hash"));
    assert_eq!(state.runtime, json!({}));
    assert_eq!(
        state.plan_hash_history,
        vec!["checkpoint-plan-hash".to_string()]
    );
    assert!(checkpoint_extensions.is_none());
    assert!(pending_resume_traces.is_empty());
    let checkpoint_plan = checkpoint_plan.expect("checkpoint plan");
    assert_eq!(checkpoint_plan.schema, "ais-plan/0.0.3");
    assert_eq!(checkpoint_plan.nodes.len(), 2);
    let ids = checkpoint_plan
        .nodes
        .iter()
        .filter_map(|node| node.get("id"))
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["seg_1__q1", "seg_1__q2"]);
    assert_eq!(
        checkpoint_plan.nodes[0].pointer("/execution/type"),
        Some(&json!("evm_read"))
    );
    assert_eq!(
        checkpoint_plan.nodes[0].pointer("/writes/0/path"),
        Some(&json!("nodes.seg_1__q1.outputs"))
    );
    assert_eq!(
        checkpoint_plan.nodes[1].pointer("/execution/type"),
        Some(&json!("evm_read"))
    );
    assert_eq!(
        checkpoint_plan.nodes[1].pointer("/writes/0/path"),
        Some(&json!("nodes.seg_1__q2.outputs"))
    );
}

#[test]
fn checkpoint_extensions_resume_core_roundtrip_restores_input_store_only() {
    let mut store = super::InputStore::default();
    store.upsert_seed(
        "owner",
        json!("0x1111111111111111111111111111111111111111"),
        "runtime.inputs.owner",
    );
    store.upsert_seed(
        "inputs.token.decimals",
        json!("6"),
        "runtime.inputs.token.decimals",
    );
    store.upsert(
        "inputs.native_balance",
        json!("123.45"),
        super::input_store::InputValueMeta {
            source: "query".to_string(),
            source_priority: 90,
            provenance: Some("segment_store.seg_1.q_balance".to_string()),
            confidence: Some(0.99),
            layer: super::input_store::InputValueLayer::Observed,
            stability: super::input_store::InputValueStability::Volatile,
            observed_at_ms: Some(1_710_000_123_456),
        },
    );
    let decoded = super::checkpoint_ext::AgentCheckpointExtensions::decode(None);
    let extensions = decoded.encode_updated_with_runtime_facts(
        None,
        &store,
        &super::runtime_facts_store::RuntimeFactsStore::default(),
    );
    let extensions_value = Value::Object(extensions.clone());
    assert!(extensions.get("resume_core").is_some());
    assert_eq!(
        extensions_value.pointer("/resume_core/input_store/entries/token.decimals/value"),
        Some(&json!(6))
    );
    assert_eq!(
        extensions_value.pointer("/resume_core/input_store/entries/token.decimals/meta/provenance"),
        Some(&json!("runtime.inputs.token.decimals"))
    );
    assert_eq!(
        extensions_value.pointer("/resume_core/input_store/entries/native_balance/meta/layer"),
        Some(&json!("observed"))
    );
    assert_eq!(
        extensions_value.pointer("/resume_core/input_store/entries/native_balance/meta/stability"),
        Some(&json!("volatile"))
    );
    assert_eq!(
        extensions_value.pointer("/resume_core/input_store/entries/native_balance/meta/observed_at_ms"),
        Some(&json!(1_710_000_123_456u64))
    );
    assert!(extensions.get("todo_progress").is_none());
    assert!(extensions.get("intent_facts").is_none());
    assert!(extensions.get("derived_projections").is_none());

    let mut restored_runtime = json!({});
    let restored_extensions =
        super::decode_agent_checkpoint_extensions(&mut restored_runtime, Some(&extensions), false);
    let restored_store = restored_extensions
        .input_store()
        .cloned()
        .expect("fact store restored");
    let restored_owner = restored_store.get("owner").expect("owner fact");
    assert_eq!(
        restored_owner.value.as_str(),
        Some("0x1111111111111111111111111111111111111111")
    );
    assert_eq!(restored_owner.meta.layer, super::input_store::InputValueLayer::Seed);
    assert_eq!(
        restored_owner.meta.source,
        "seed"
    );
    assert_eq!(
        restored_owner.meta.provenance.as_deref(),
        Some("runtime.inputs.owner")
    );
    assert_eq!(
        restored_owner.meta.stability,
        super::input_store::InputValueStability::Unknown
    );
    assert_eq!(restored_owner.meta.observed_at_ms, None);
    assert_eq!(
        restored_store
            .get("token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    let restored_balance = restored_store
        .get("native_balance")
        .expect("native_balance restored");
    assert_eq!(restored_balance.meta.layer, super::input_store::InputValueLayer::Observed);
    assert_eq!(
        restored_balance.meta.stability,
        super::input_store::InputValueStability::Volatile
    );
    assert_eq!(restored_balance.meta.observed_at_ms, Some(1_710_000_123_456));
    assert_eq!(
        restored_store
            .get("inputs.token.decimals")
            .and_then(|entry| entry.value.as_i64()),
        Some(6)
    );
    assert!(restored_runtime.pointer("/agent/todo_progress").is_none());
    assert!(restored_runtime.pointer("/agent/intent_grounding/intent_facts").is_none());
    let projection = restored_store.to_runtime_projection();
    if let Some(inputs) = projection.pointer("/inputs") {
        restored_runtime
            .as_object_mut()
            .expect("runtime object")
            .insert("inputs".to_string(), inputs.clone());
    }
    assert_eq!(
        restored_runtime.pointer("/inputs/owner"),
        Some(&json!("0x1111111111111111111111111111111111111111"))
    );
    assert_eq!(
        restored_runtime.pointer("/inputs/token/decimals"),
        Some(&json!(6))
    );
}

#[test]
fn decode_agent_checkpoint_extensions_ignores_legacy_derived_projection_section() {
    let mut runtime = json!({
        "agent": {
            "todo_progress": {
                "current_todo": {"id": "todo_runtime"}
            },
            "intent_grounding": {
                "intent_facts": {"recipient": "0xruntime"}
            }
        }
    });
    let extensions = Map::from_iter([(
        "derived_projections".to_string(),
        json!({
            "todo_progress": {
                "current_todo": {"id": "todo_derived"}
            },
            "intent_facts": {"recipient": "0xderived"}
        }),
    )]);

    let decoded =
        super::decode_agent_checkpoint_extensions(&mut runtime, Some(&extensions), false);

    assert_eq!(
        runtime.pointer("/agent/todo_progress/current_todo/id"),
        Some(&json!("todo_runtime"))
    );
    assert_eq!(
        runtime.pointer("/agent/intent_grounding/intent_facts/recipient"),
        Some(&json!("0xruntime"))
    );
    assert!(decoded.input_store().is_none());
    assert!(decoded.runtime_facts_store().is_none());
}

#[test]
fn build_initial_input_store_does_not_treat_runtime_snapshot_intent_facts_as_inputs() {
    let mut runtime = json!({
        "agent": {
            "intent_grounding": {
                "intent_facts": {
                    "recipient": "0xruntime",
                    "amount": "5"
                }
            }
        }
    });
    let extensions = Map::from_iter([(
        "derived_projections".to_string(),
        json!({
            "intent_facts": {
                "recipient": "0xderived",
                "amount": "1"
            }
        }),
    )]);
    let _decoded =
        super::decode_agent_checkpoint_extensions(&mut runtime, Some(&extensions), false);
    let config = RunnerConfig {
        schema: "ais-runner-config/test".to_string(),
        engine: RunnerEngineConfig::default(),
        llm: None,
        chains: std::collections::BTreeMap::new(),
        plugins: RunnerPluginsConfig::default(),
    };
    let input_store =
        super::build_initial_input_store(&runtime, &config, &[]).expect("build input store");

    assert!(input_store.get("recipient").is_none());
    assert!(input_store.get("amount").is_none());
}

#[test]
fn maybe_save_checkpoint_persists_inferred_missing_input_pause_reason() {
    let checkpoint_path = write_temp_file("pause-contract-checkpoint", "");
    let command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };
    let state = EngineRunnerState {
        runtime: json!({
            "agent": {
                "missing_required_input": {
                    "reason_code": "missing_required_input",
                    "consumed": false,
                    "questions": [{"id":"inputs.owner","question":"owner?","required":true}]
                }
            }
        }),
        ..EngineRunnerState::default()
    };
    let plan = super::empty_plan_document();

    super::maybe_save_checkpoint(
        &command,
        "run-test",
        "plan-hash",
        &plan,
        &state,
        &state.runtime,
        &RunnerCheckpointLedger::default(),
        None,
        &crate::audit_contract::AuditStreamAttempt::fresh(),
    )
    .expect("save checkpoint");

    let checkpoint = load_checkpoint_from_path(&checkpoint_path).expect("load checkpoint");
    assert_eq!(
        checkpoint.engine_state.paused_reason.as_deref(),
        Some("missing_required_input")
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("attempt_id"))
            .and_then(Value::as_str),
        Some("attempt-1")
    );
    assert_eq!(
        checkpoint
            .extensions
            .get("resume_core")
            .and_then(|value| value.get("audit_stream"))
            .and_then(|value| value.get("seq_scope"))
            .and_then(Value::as_str),
        Some("attempt_local")
    );
}

#[test]
fn agent_write_event_sinks_without_persisted_engine_sink_does_not_advance_watermark() {
    let command = AgentCommand {
        plan: None,
        intent: Some("transfer".to_string()),
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: None,
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };
    let events = vec![ais_engine::EngineEventRecord::new(
        "run-test",
        7,
        "2026-03-07T00:00:00Z",
        ais_engine::EngineEvent::new(ais_engine::EngineEventType::EnginePaused),
    )];
    let mut audit_attempt = crate::audit_contract::AuditStreamAttempt::fresh();

    super::write_event_sinks(&command, &events, &mut audit_attempt).expect("write events");

    assert_eq!(audit_attempt.last_event_seq, None);
    assert_eq!(audit_attempt.last_event_ts, None);
}

#[test]
fn segmented_checkpoint_resume_keeps_need_user_confirm_pause_in_real_flow() {
    let plan = plan_requiring_user_confirm("transfer-1");
    let checkpoint_path = write_temp_file("agent-checkpoint-path", "");
    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };
    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));
    let mut state = EngineRunnerState::default();
    let mut options = EngineRunnerOptions::default();
    options.policy.thresholds.max_risk_level = Some(0);
    let run = run_plan_once(
        "run-checkpoint-pause",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &options,
    );
    assert_eq!(run.status, EngineRunStatus::Paused);
    assert_eq!(
        state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );

    let plan_hash = super::hash_plan(&plan).expect("plan hash");
    super::maybe_save_checkpoint(
        &command,
        "run-checkpoint-pause",
        plan_hash.as_str(),
        &plan,
        &state,
        &state.runtime,
        &crate::checkpoint_ledger::RunnerCheckpointLedger::default(),
        None,
        &crate::audit_contract::AuditStreamAttempt::fresh(),
    )
    .expect("save checkpoint");
    let (restored_state, resumed, checkpoint_plan, checkpoint_hash, _, _, _) =
        super::load_or_init_state(&command, plan_hash.as_str(), json!({}))
            .expect("load checkpoint");
    assert!(resumed);
    assert_eq!(
        restored_state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );
    assert!(checkpoint_plan.is_none());
    assert_eq!(checkpoint_hash.as_deref(), Some(plan_hash.as_str()));

    let checkpoint =
        ais_engine::load_checkpoint_from_path(checkpoint_path.as_path()).expect("load checkpoint");
    assert_eq!(
        checkpoint.engine_state.paused_reason.as_deref(),
        Some("need_user_confirm:transfer-1")
    );
}

#[test]
fn load_or_init_state_restores_approved_nodes_and_skips_reconfirm_on_resume() {
    let checkpoint_path = write_temp_file("approved-restore-checkpoint", "");
    let mut checkpoint = create_checkpoint_document(
        "run-approved-restore",
        "plan-hash-restore",
        CheckpointEngineState {
            paused_reason: Some("need_user_confirm:transfer-1".to_string()),
            ..CheckpointEngineState::default()
        },
        Some(json!({})),
        None,
        None,
    );
    checkpoint.approvals_ledger = vec![ais_engine::CheckpointApprovalLedgerEntry {
        node_id: "transfer-1".to_string(),
        confirmation_hash: None,
        decision: "approve".to_string(),
        reason_code: None,
        decided_at: "2026-02-24T00:00:00Z".to_string(),
    }];
    save_checkpoint_to_path(&checkpoint_path, &checkpoint).expect("save checkpoint");

    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };

    let (mut restored_state, resumed, _, checkpoint_hash, _, _, _) =
        super::load_or_init_state(&command, "plan-hash-restore", json!({}))
            .expect("load checkpoint");
    assert!(resumed);
    assert_eq!(checkpoint_hash.as_deref(), Some("plan-hash-restore"));
    assert_eq!(
        restored_state.approved_node_ids,
        vec!["transfer-1".to_string()]
    );

    let plan = plan_requiring_user_confirm("transfer-1");
    let mut router = RouterExecutor::new();
    router.register("test-exec", "test", Box::new(TestExecutor));
    let mut options = EngineRunnerOptions::default();
    options.policy.thresholds.max_risk_level = Some(0);

    let run = run_plan_once(
        "run-approved-restore",
        &plan,
        &mut restored_state,
        &router,
        &DefaultSolver,
        &[],
        &options,
    );
    assert_eq!(run.status, EngineRunStatus::Completed);
    assert!(run
        .events
        .iter()
        .all(|record| { record.event.event_type != ais_engine::EngineEventType::NeedUserConfirm }));
    assert!(restored_state
        .completed_node_ids
        .contains(&"transfer-1".to_string()));
}

#[test]
fn resume_skips_confirmed_write_with_same_confirmation_hash() {
    use ais_engine::{CheckpointSideEffectRecord, EngineCommand, EngineCommandType, ExecutorOutput};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct CountingWriteExecutor {
        execute_count: Arc<AtomicUsize>,
    }

    impl ais_engine::Executor for CountingWriteExecutor {
        fn execute(&self, _node: &Value, _runtime: &mut Value) -> Result<ExecutorOutput, String> {
            self.execute_count.fetch_add(1, Ordering::SeqCst);
            Ok(ExecutorOutput {
                result: json!({"ok": true}),
                writes: Map::new(),
                side_effects: vec![CheckpointSideEffectRecord {
                    schema: Some("ais-side-effect-record/0.1.0".to_string()),
                    idempotency_key: "tx:transfer-1:0xtx1".to_string(),
                    node_id: "transfer-1".to_string(),
                    effect_type: "tx".to_string(),
                    chain: Some("test".to_string()),
                    execution_type: Some("test-exec".to_string()),
                    tx_hash: Some("0xtx1".to_string()),
                    nonce: None,
                    provider_ref: None,
                    reason_code: None,
                    details: None,
                    status: "confirmed".to_string(),
                    observed_at: "2026-02-24T00:00:02Z".to_string(),
                }],
            })
        }
    }

    let checkpoint_path = write_temp_file("resume-skip-confirmed-write", "");
    let command = AgentCommand {
        plan: Some(PathBuf::from("ignored.plan.json")),
        intent: None,
        intent_file: None,
        workspace: None,
        config: PathBuf::from("ignored.config.yaml"),
        pack: None,
        runtime: None,
        events_jsonl: None,
        trace: None,
        agent_trace_jsonl: None,
        checkpoint: Some(checkpoint_path.clone()),
        profile: AgentProfile::Standard,
        llm_script_jsonl: None,
        verbose: false,
        verbose_llm: false,
        approvals_mode: None,
        max_iterations: None,
        max_planner_rounds: None,
        max_tool_rounds: None,
        max_index_candidates: None,
        planner_context_token_budget: None,
        llm_transcript_path: None,
        llm_transcript_append: false,
        format: OutputFormat::Text,
    };
    let plan = plan_requiring_user_confirm("transfer-1");
    let mut options = EngineRunnerOptions::default();
    options.policy.thresholds.max_risk_level = Some(0);
    let mut state = EngineRunnerState::default();
    let execute_count = Arc::new(AtomicUsize::new(0));
    let mut router = RouterExecutor::new();
    router.register(
        "test-exec",
        "test",
        Box::new(CountingWriteExecutor {
            execute_count: execute_count.clone(),
        }),
    );

    let first = run_plan_once(
        "run-resume-skip-confirmed-write",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[],
        &options,
    );
    assert_eq!(first.status, EngineRunStatus::Paused);
    let confirmation_hash = first
        .events
        .iter()
        .find(|record| record.event.event_type == ais_engine::EngineEventType::NeedUserConfirm)
        .and_then(|record| record.event.data.get("details"))
        .and_then(Value::as_object)
        .and_then(|details| details.get("confirmation_hash"))
        .and_then(Value::as_str)
        .expect("confirmation_hash")
        .to_string();

    let approve = EngineCommandEnvelope::new(EngineCommand {
        id: "cmd-approve-1".to_string(),
        command_type: EngineCommandType::UserConfirm,
        data: Map::from_iter([
            ("node_id".to_string(), json!("transfer-1")),
            ("decision".to_string(), json!("approve")),
        ]),
    });
    let second = run_plan_once(
        "run-resume-skip-confirmed-write",
        &plan,
        &mut state,
        &router,
        &DefaultSolver,
        &[approve],
        &options,
    );
    assert_eq!(second.status, EngineRunStatus::Completed);
    assert_eq!(execute_count.load(Ordering::SeqCst), 1);

    let mut ledger = crate::checkpoint_ledger::RunnerCheckpointLedger::default();
    ledger.absorb_events(first.events.as_slice());
    ledger.mark_approved_nodes(
        &state.approved_node_ids,
        "2026-02-24T00:00:01Z",
    );
    ledger.absorb_events(second.events.as_slice());
    ledger.mark_approved_nodes(
        &state.approved_node_ids,
        "2026-02-24T00:00:03Z",
    );

    let plan_hash = super::hash_plan(&plan).expect("plan hash");
    super::maybe_save_checkpoint(
        &command,
        "run-resume-skip-confirmed-write",
        plan_hash.as_str(),
        &plan,
        &state,
        &state.runtime,
        &ledger,
        None,
        &crate::audit_contract::AuditStreamAttempt::fresh(),
    )
    .expect("save checkpoint");

    let (
        mut restored_state,
        resumed,
        _,
        checkpoint_hash,
        restored_ledger,
        _,
        pending_resume_traces,
    ) = super::load_or_init_state(&command, plan_hash.as_str(), json!({}))
        .expect("load checkpoint");
    assert!(resumed);
    assert_eq!(checkpoint_hash.as_deref(), Some(plan_hash.as_str()));
    assert!(restored_state.approved_node_ids.is_empty());
    assert!(restored_state
        .completed_node_ids
        .contains(&"transfer-1".to_string()));
    assert_eq!(restored_ledger.confirmed_write_reuses().len(), 1);
    assert_eq!(
        restored_ledger.confirmed_write_reuses()[0].confirmation_hash,
        confirmation_hash
    );
    assert_eq!(restored_ledger.side_effects().len(), 1);
    assert_eq!(
        restored_ledger.side_effects()[0].status,
        ais_engine::SIDE_EFFECT_STATUS_CONFIRMED
    );
    assert_eq!(pending_resume_traces.len(), 2);
    assert_eq!(pending_resume_traces[0].phase, "resume");
    assert_eq!(pending_resume_traces[0].event, "side_effect_reused");
    assert_eq!(pending_resume_traces[1].event, "resume_skip_confirmed_write");

    let resume = run_plan_once(
        "run-resume-skip-confirmed-write",
        &plan,
        &mut restored_state,
        &router,
        &DefaultSolver,
        &[],
        &options,
    );
    assert_eq!(resume.status, EngineRunStatus::Completed);
    assert_eq!(
        execute_count.load(Ordering::SeqCst),
        1,
        "executor should not be called again after resume"
    );
    assert!(resume.events.iter().all(|record| {
        record.event.event_type != ais_engine::EngineEventType::SideEffectObserved
    }));
}

#[test]
fn segmented_completed_run_checkpoint_todo_progress_matches_terminal_state() {
    let checkpoint_path = write_temp_file("agent-segmented-completed-checkpoint", "{}");
    let _ = fs::remove_file(&checkpoint_path);
    let llm_script = [
        serde_json::to_string(&json!({
            "assistant_content":"begin segmented session",
            "tool_calls":[{
                "id":"tool-begin",
                "name":"plan.begin",
                "arguments":{
                    "session_id":"sess-checkpoint-terminal",
                    "snapshot_hash":"llm-snapshot-placeholder",
                    "cursor":"cursor-0",
                    "limits":{"max_rounds":4,"max_segments":1}
                }
            }]
        }))
        .expect("script line 1"),
        serde_json::to_string(&json!({
            "assistant_content":"ground intent",
            "tool_calls":[{
                "id":"tool-ground",
                "name":"plan.ground_intent",
                "arguments":{
                    "status":"proposed",
                    "ready_for_todos": true,
                    "resolved_inputs": {}
                }
            }]
        }))
        .expect("script line 2"),
        serde_json::to_string(&json!({
            "assistant_content":"propose todos",
            "tool_calls":[{
                "id":"tool-todos",
                "name":"plan.propose_todos",
                "arguments":{
                    "status":"proposed",
                    "todos":[{"title":"assert terminal checkpoint sync"}]
                }
            }]
        }))
        .expect("script line 3"),
        serde_json::to_string(&json!({
            "assistant_content":"check segment",
            "tool_calls":[{
                "id":"tool-check",
                "name":"plan.check_segment",
                "arguments":{
                    "segment":{
                        "segment_id":"seg-terminal",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":true,
                        "summary":"single assert step",
                        "steps":[{
                            "id":"assert_true",
                            "kind":"assert",
                            "inputs":{"condition":{"cel":"true"}}
                        }],
                        "extensions":{}
                    }
                }
            }]
        }))
        .expect("script line 4"),
        serde_json::to_string(&json!({
            "assistant_content":"propose segment",
            "tool_calls":[{
                "id":"tool-propose",
                "name":"plan.propose_segment",
                "arguments":{
                    "status":"proposed",
                    "done":true,
                    "cursor_next":"cursor-1",
                    "summary":"single assert step",
                    "segment":{
                        "segment_id":"seg-terminal",
                        "cursor_in":"cursor-0",
                        "cursor_out":"cursor-1",
                        "done":true,
                        "summary":"single assert step",
                        "steps":[{
                            "id":"assert_true",
                            "kind":"assert",
                            "inputs":{"condition":{"cel":"true"}}
                        }],
                        "extensions":{}
                    },
                    "issues":[]
                }
            }]
        }))
        .expect("script line 5"),
    ]
    .join("\n");
    let llm_script_path = write_temp_file("agent-segmented-completed-script", llm_script.as_str());

    let mut command =
        build_segmented_demo_command(llm_script_path, Some(checkpoint_path.clone()), None);
    command.approvals_mode = Some(crate::cli::ApprovalsMode::Yolo);
    let parsed = parse_agent_output_json(&command);
    assert_eq!(parsed.get("status").and_then(Value::as_str), Some("completed"));

    let checkpoint_text = fs::read_to_string(checkpoint_path).expect("checkpoint file");
    let checkpoint: Value = serde_json::from_str(checkpoint_text.as_str()).expect("checkpoint");
    assert_eq!(
        checkpoint.pointer("/runtime_snapshot/agent/todo_progress/schema"),
        Some(&json!("ais-agent-todo-progress/0.0.1"))
    );
    assert_ne!(
        checkpoint.pointer("/runtime_snapshot/agent/todo_progress/current_todo/status"),
        Some(&json!("in_progress"))
    );
    assert_eq!(
        checkpoint.pointer("/runtime_snapshot/agent/todo_progress/progress/in_progress"),
        Some(&json!(0))
    );
    assert!(
        checkpoint
            .pointer("/runtime_snapshot/agent/todo_progress/progress/done")
            .and_then(Value::as_u64)
            .is_some_and(|value| value >= 1)
    );
}
