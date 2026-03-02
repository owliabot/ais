use crate::checkpoint_ledger::RunnerCheckpointLedger;
use crate::cli::{OutputFormat, PlanCommand, PlanDiffCommand, ReplayCommand, WorkflowCommand};
use crate::config::{build_router_executor_for_plan, load_runner_config};
use crate::error::RunnerError;
use crate::io::load_workspace_documents_excluding;
use ais_core::StructuredIssue;
use ais_engine::events::wall_clock_timestamp_rfc3339;
use ais_engine::{
    apply_command_with_dedupe, create_checkpoint_document, decode_command_jsonl_line,
    diff_plans_json, diff_plans_text, encode_event_jsonl_line, encode_trace_jsonl_line,
    load_checkpoint_from_path, replay_from_checkpoint, replay_trace_jsonl, run_plan_once,
    save_checkpoint_to_path, CheckpointEngineState, CommandDeduper, DefaultSolver,
    EngineCommandEnvelope, EngineCommandType, EngineEvent, EngineEventRecord, EngineEventStream,
    EngineEventType, EngineRunStatus, EngineRunnerOptions, EngineRunnerState, ReplayOptions,
    TraceRedactOptions, SIDE_EFFECT_STATUS_CONFIRMED, SIDE_EFFECT_STATUS_REVERTED,
};
use ais_sdk::{
    compile_workflow, dry_run_json, dry_run_text, evaluate_value_ref_with_options,
    parse_document_with_options, validate_workflow_document, validate_workspace_references,
    AisDocument, CompileWorkflowOptions, CompileWorkflowResult, DocumentFormat,
    ParseDocumentOptions, ResolverContext, ValueRef, ValueRefEvalOptions, WorkspaceDocuments,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn execute_run_plan(command: &PlanCommand) -> Result<String, RunnerError> {
    let plan_text = fs::read_to_string(&command.plan).map_err(|source| RunnerError::ReadFile {
        path: command.plan.display().to_string(),
        source,
    })?;
    let parsed = parse_document_with_options(
        plan_text.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .map_err(|issues| RunnerError::PlanParse(format!("{issues:?}")))?;

    let plan = match parsed {
        AisDocument::Plan(plan) => plan,
        _ => return Err(RunnerError::NotPlanDocument),
    };

    let runtime = match &command.runtime {
        Some(path) => {
            let runtime_text =
                fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
                    path: path.display().to_string(),
                    source,
                })?;
            parse_runtime_value(runtime_text.as_str())?
        }
        None => Value::Object(serde_json::Map::new()),
    };

    let output = if command.dry_run {
        let context = ResolverContext::with_runtime(runtime);
        match command.format {
            OutputFormat::Text => dry_run_text(&plan, &context, &ValueRefEvalOptions::default()),
            OutputFormat::Json => serde_json::to_string_pretty(&dry_run_json(
                &plan,
                &context,
                &ValueRefEvalOptions::default(),
            ))?,
        }
    } else {
        execute_plan_with_engine(command, &plan, runtime)?.rendered
    };

    Ok(output)
}

pub fn execute_run_workflow(command: &WorkflowCommand) -> Result<String, RunnerError> {
    let workflow_text =
        fs::read_to_string(&command.workflow).map_err(|source| RunnerError::ReadFile {
            path: command.workflow.display().to_string(),
            source,
        })?;
    let workflow: AisDocument = parse_document_with_options(
        workflow_text.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .map_err(|issues| RunnerError::WorkflowParse(format!("{issues:?}")))?;
    let workflow = match workflow {
        AisDocument::Workflow(workflow) => workflow,
        _ => {
            return Err(RunnerError::WorkflowParse(
                "workflow file must be AIS workflow document".to_string(),
            ))
        }
    };

    let workspace_root = match &command.workspace {
        Some(path) => path.clone(),
        None => command
            .workflow
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf(),
    };

    let mut loaded = load_workspace_documents_excluding(
        workspace_root.as_path(),
        std::slice::from_ref(&command.workflow),
    )
    .map_err(|issues| RunnerError::WorkspaceLoad(format!("{issues:?}")))?;
    loaded.workflows.push(workflow.clone());

    let mut issues = validate_workspace_references(WorkspaceDocuments {
        protocols: &loaded.protocols,
        packs: &loaded.packs,
        workflows: &loaded.workflows,
    });
    issues.extend(validate_workflow_document(&workflow));
    StructuredIssue::sort_stable(&mut issues);
    if !issues.is_empty() {
        let workspace_issues = issues
            .iter()
            .filter(|issue| {
                issue
                    .reference
                    .as_deref()
                    .is_some_and(|reference| reference.starts_with("workspace."))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !workspace_issues.is_empty() {
            return Err(RunnerError::WorkspaceValidate(format!(
                "{workspace_issues:?}"
            )));
        }
        return Err(RunnerError::WorkflowValidate(format!("{issues:?}")));
    }

    let mut compile_context = ResolverContext::new();
    for protocol in loaded.protocols.iter().cloned() {
        compile_context.register_protocol(protocol);
    }

    let plan = match compile_workflow(
        &workflow,
        &compile_context,
        &CompileWorkflowOptions::default(),
    ) {
        CompileWorkflowResult::Ok { plan } => plan,
        CompileWorkflowResult::Err { issues } => {
            return Err(RunnerError::WorkflowCompile(format!("{issues:?}")));
        }
    };

    let mut runtime = match &command.runtime {
        Some(path) => {
            let runtime_text =
                fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
                    path: path.display().to_string(),
                    source,
                })?;
            parse_runtime_value(runtime_text.as_str())?
        }
        None => Value::Object(serde_json::Map::new()),
    };
    merge_workflow_input_defaults(&mut runtime, &workflow);

    if command.dry_run {
        let context = ResolverContext::with_runtime(runtime);
        return match command.format {
            OutputFormat::Json => {
                let report = json!({
                    "schema": "ais-runner-run-workflow/0.0.1",
                    "workflow": command.workflow.display().to_string(),
                    "workspace": workspace_root.display().to_string(),
                    "documents": {
                        "protocols": loaded.protocols.len(),
                        "packs": loaded.packs.len(),
                        "workflows": loaded.workflows.len(),
                        "plans": loaded.plans.len(),
                    },
                    "plan": plan,
                    "dry_run": dry_run_json(
                        &plan,
                        &context,
                        &ValueRefEvalOptions::default(),
                    ),
                    "issues": [],
                });
                serde_json::to_string_pretty(&report).map_err(RunnerError::from)
            }
            OutputFormat::Text => Ok(format!(
                "AIS run workflow (dry-run)\nworkflow: {}\nworkspace: {}\ncompiled_plan_nodes: {}\ndocuments: protocols={} packs={} workflows={} plans={}\n{}",
                command.workflow.display(),
                workspace_root.display(),
                plan.nodes.len(),
                loaded.protocols.len(),
                loaded.packs.len(),
                loaded.workflows.len(),
                loaded.plans.len(),
                dry_run_text(&plan, &context, &ValueRefEvalOptions::default()),
            )),
        };
    }

    let run_result = execute_plan_with_engine(
        &PlanCommand {
            plan: command.workflow.clone(),
            config: command.config.clone(),
            runtime: command.runtime.clone(),
            dry_run: false,
            events_jsonl: command.events_jsonl.clone(),
            trace: command.trace.clone(),
            checkpoint: command.checkpoint.clone(),
            commands_stdin_jsonl: command.commands_stdin_jsonl,
            verbose: command.verbose,
            format: command.format.clone(),
        },
        &plan,
        runtime,
    )?;
    if let Some(path) = &command.outputs {
        let outputs = evaluate_workflow_outputs(&workflow, &run_result.runtime)?;
        let payload = serde_json::to_string_pretty(&json!({
            "schema": "ais-runner-workflow-outputs/0.0.1",
            "outputs": outputs,
        }))?;
        fs::write(path, payload).map_err(|source| RunnerError::WriteFile {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(run_result.rendered)
}

pub fn execute_plan_diff(command: &PlanDiffCommand) -> Result<String, RunnerError> {
    let before = read_plan_document(command.before.as_path(), RunnerError::PlanDiffBeforeParse)?;
    let after = read_plan_document(command.after.as_path(), RunnerError::PlanDiffAfterParse)?;

    match command.format {
        OutputFormat::Text => Ok(diff_plans_text(&before, &after)),
        OutputFormat::Json => serde_json::to_string_pretty(&diff_plans_json(&before, &after))
            .map_err(RunnerError::from),
    }
}

pub fn execute_replay(command: &ReplayCommand) -> Result<String, RunnerError> {
    let options = ReplayOptions {
        until_node: command.until_node.clone(),
        max_steps: 128,
    };

    if let Some(trace_path) = &command.trace_jsonl {
        let trace =
            fs::read_to_string(trace_path).map_err(|source| RunnerError::ReplayTraceRead {
                path: trace_path.display().to_string(),
                source,
            })?;
        let result = replay_trace_jsonl(trace.as_str(), &options)
            .map_err(|error| RunnerError::ReplayTraceDecode(error.to_string()))?;
        return render_replay_output(
            command,
            &result.events,
            replay_status_label(result.status),
            &result.completed_node_ids,
            result.paused_reason.as_deref(),
        );
    }

    if let Some(checkpoint_path) = &command.checkpoint {
        let plan_path = command
            .plan
            .as_ref()
            .ok_or(RunnerError::ReplayMissingPlan)?;
        let config_path = command
            .config
            .as_ref()
            .ok_or(RunnerError::ReplayMissingConfig)?;
        let plan = read_plan_document(plan_path.as_path(), RunnerError::ReplayPlanParse)?;
        let config = load_runner_config(config_path.as_path())
            .map_err(|error| RunnerError::ConfigLoad(error.to_string()))?;
        let router = build_router_executor_for_plan(&plan, &config)
            .map_err(RunnerError::ConfigInvalidForPlan)?;
        let checkpoint = load_checkpoint_from_path(checkpoint_path).map_err(|error| {
            RunnerError::CheckpointLoad {
                path: checkpoint_path.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        let result = replay_from_checkpoint(
            &plan,
            &checkpoint,
            &router,
            &DefaultSolver,
            &EngineRunnerOptions::default(),
            &options,
        );
        return render_replay_output(
            command,
            &result.events,
            replay_status_label(result.status),
            &result.completed_node_ids,
            result.paused_reason.as_deref(),
        );
    }

    Err(RunnerError::ReplayInputRequired)
}

fn parse_runtime_value(input: &str) -> Result<Value, RunnerError> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str::<Value>(input)
            .map_err(|error| RunnerError::RuntimeParse(error.to_string()))
    } else {
        serde_yaml::from_str::<Value>(input)
            .map_err(|error| RunnerError::RuntimeParse(error.to_string()))
    }
}

fn merge_workflow_input_defaults(runtime: &mut Value, workflow: &ais_sdk::WorkflowDocument) {
    if !runtime.is_object() {
        *runtime = Value::Object(serde_json::Map::new());
    }
    let Some(runtime_object) = runtime.as_object_mut() else {
        return;
    };
    let runtime_inputs = runtime_object
        .entry("inputs".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !runtime_inputs.is_object() {
        *runtime_inputs = Value::Object(serde_json::Map::new());
    }
    let Some(runtime_inputs_object) = runtime_inputs.as_object_mut() else {
        return;
    };
    for (key, input_spec) in &workflow.inputs {
        let Some(default_value) = input_spec
            .as_object()
            .and_then(|object| object.get("default"))
            .cloned()
        else {
            continue;
        };
        runtime_inputs_object
            .entry(key.clone())
            .or_insert(default_value);
    }
}

struct PlanExecutionResult {
    rendered: String,
    runtime: Value,
}

pub(crate) struct ReplacePlanCommandProcessing {
    pub(crate) forward_commands: Vec<EngineCommandEnvelope>,
    pub(crate) events: Vec<EngineEventRecord>,
    pub(crate) plan_replaced: bool,
    pub(crate) pause_after_processing: bool,
}

fn execute_plan_with_engine(
    command: &PlanCommand,
    plan: &ais_sdk::PlanDocument,
    runtime: Value,
) -> Result<PlanExecutionResult, RunnerError> {
    let config_path = command
        .config
        .as_ref()
        .ok_or(RunnerError::MissingRunnerConfig)?;
    let config = load_runner_config(config_path.as_path())
        .map_err(|error| RunnerError::ConfigLoad(error.to_string()))?;
    let mut queued_commands = Some(read_commands_from_stdin(command.commands_stdin_jsonl)?);
    let mut active_plan = plan.clone();
    let mut active_plan_hash = hash_plan(&active_plan)?;
    let mut resumed_from_checkpoint = false;
    let mut checkpoint_ledger = RunnerCheckpointLedger::default();
    let mut state = if let Some(checkpoint_path) = &command.checkpoint {
        let checkpoint_usable = fs::metadata(checkpoint_path)
            .map(|meta| meta.is_file() && meta.len() > 0)
            .unwrap_or(false);
        if !checkpoint_usable {
            EngineRunnerState {
                runtime,
                plan_hash_history: vec![active_plan_hash.clone()],
                ..EngineRunnerState::default()
            }
        } else {
            match load_checkpoint_from_path(checkpoint_path) {
                Ok(checkpoint) => {
                    resumed_from_checkpoint = true;
                    checkpoint_ledger = RunnerCheckpointLedger::from_checkpoint(
                        &checkpoint.approvals_ledger,
                        &checkpoint.side_effects,
                    );
                    if checkpoint.plan_hash != active_plan_hash {
                        let Some(plan_snapshot) = checkpoint.plan_snapshot.clone() else {
                            return Err(RunnerError::CheckpointLoad {
                                path: checkpoint_path.display().to_string(),
                                reason: "checkpoint plan hash mismatch".to_string(),
                            });
                        };
                        active_plan = serde_json::from_value(plan_snapshot).map_err(|error| {
                            RunnerError::CheckpointLoad {
                                path: checkpoint_path.display().to_string(),
                                reason: format!("checkpoint plan snapshot decode failed: {error}"),
                            }
                        })?;
                        active_plan_hash = checkpoint.plan_hash.clone();
                    }
                    EngineRunnerState {
                        runtime: checkpoint
                            .runtime_snapshot
                            .unwrap_or_else(|| runtime.clone()),
                        completed_node_ids: checkpoint.engine_state.completed_node_ids,
                        approved_node_ids: Vec::new(),
                        seen_command_ids: checkpoint.engine_state.seen_command_ids,
                        paused_reason: checkpoint.engine_state.paused_reason,
                        pending_retries: checkpoint.engine_state.pending_retries,
                        plan_epoch: checkpoint.engine_state.plan_epoch,
                        plan_hash_history: checkpoint.engine_state.plan_hash_history,
                        next_seq: 0,
                    }
                }
                Err(ais_engine::CheckpointStoreError::Io(_)) => EngineRunnerState {
                    runtime,
                    plan_hash_history: vec![active_plan_hash.clone()],
                    ..EngineRunnerState::default()
                },
                Err(error) => {
                    return Err(RunnerError::CheckpointLoad {
                        path: checkpoint_path.display().to_string(),
                        reason: error.to_string(),
                    });
                }
            }
        }
    } else {
        EngineRunnerState {
            runtime,
            plan_hash_history: vec![active_plan_hash.clone()],
            ..EngineRunnerState::default()
        }
    };
    if state.plan_hash_history.is_empty() {
        state.plan_hash_history.push(active_plan_hash.clone());
    }
    let mut router = build_router_executor_for_plan(&active_plan, &config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    checkpoint_ledger
        .reconcile_completed_from_confirmed_side_effects(&mut state.completed_node_ids);
    record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);

    let run_id = format!(
        "run-{}",
        active_plan_hash
            .get(0..12)
            .unwrap_or(active_plan_hash.as_str())
    );
    let mut all_events = Vec::<ais_engine::EngineEventRecord>::new();

    if resumed_from_checkpoint {
        if let Some(paused_reason) =
            reconcile_pending_side_effects(&mut checkpoint_ledger, &router, &mut state)?
        {
            record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
            state.paused_reason = Some(paused_reason);
            maybe_save_checkpoint(
                command,
                run_id.as_str(),
                &active_plan_hash,
                &active_plan,
                &state,
                &checkpoint_ledger,
            )?;
            let rendered = render_execution_output(
                command,
                &state,
                EngineRunStatus::Paused,
                resumed_from_checkpoint,
                0,
                &all_events,
            )?;
            return Ok(PlanExecutionResult {
                rendered,
                runtime: state.runtime,
            });
        }
    }
    let mut iteration = 0usize;
    let max_iterations = active_plan.nodes.len().saturating_mul(6).max(16);
    let options = EngineRunnerOptions::default();
    let final_status = loop {
        iteration += 1;
        if iteration > max_iterations {
            return Err(RunnerError::IterationLimitExceeded(max_iterations));
        }

        let current_commands = queued_commands.as_deref().unwrap_or(&[]);
        let processed = process_replace_plan_commands(
            run_id.as_str(),
            &config,
            current_commands,
            &options,
            &mut state,
            &mut active_plan,
            &mut active_plan_hash,
        )?;
        let run_result = run_plan_once(
            run_id.as_str(),
            &active_plan,
            &mut state,
            &router,
            &DefaultSolver,
            processed.forward_commands.as_slice(),
            &options,
        );
        queued_commands = None;
        if processed.plan_replaced {
            router = build_router_executor_for_plan(&active_plan, &config)
                .map_err(RunnerError::ConfigInvalidForPlan)?;
        }
        let mut iteration_events = processed.events;
        iteration_events.extend(run_result.events);
        checkpoint_ledger.absorb_events(&iteration_events);
        checkpoint_ledger.mark_approved_nodes(
            &state.approved_node_ids,
            wall_clock_timestamp_rfc3339().as_str(),
        );
        record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
        write_event_sinks(command, &iteration_events)?;
        all_events.extend(iteration_events);
        maybe_save_checkpoint(
            command,
            run_id.as_str(),
            &active_plan_hash,
            &active_plan,
            &state,
            &checkpoint_ledger,
        )?;
        if processed.pause_after_processing {
            break EngineRunStatus::Paused;
        }

        match run_result.status {
            EngineRunStatus::Completed => break EngineRunStatus::Completed,
            EngineRunStatus::Stopped => break EngineRunStatus::Stopped,
            EngineRunStatus::Paused => {
                if state.paused_reason.is_some() {
                    break EngineRunStatus::Paused;
                }
            }
        }
    };

    let rendered = render_execution_output(
        command,
        &state,
        final_status,
        resumed_from_checkpoint,
        iteration,
        &all_events,
    )?;
    Ok(PlanExecutionResult {
        rendered,
        runtime: state.runtime,
    })
}

pub(crate) fn process_replace_plan_commands(
    run_id: &str,
    config: &crate::config::RunnerConfig,
    commands: &[EngineCommandEnvelope],
    options: &EngineRunnerOptions,
    state: &mut EngineRunnerState,
    active_plan: &mut ais_sdk::PlanDocument,
    active_plan_hash: &mut String,
) -> Result<ReplacePlanCommandProcessing, RunnerError> {
    let mut forward_commands = Vec::<EngineCommandEnvelope>::new();
    let mut events = Vec::<EngineEventRecord>::new();
    let mut plan_replaced = false;
    let mut pause_after_processing = false;

    let mut deduper = CommandDeduper::with_seen_ids(
        options.duplicate_command_mode,
        state.seen_command_ids.clone(),
    );
    let mut stream = EngineEventStream::with_start_seq(run_id.to_string(), state.next_seq);

    for command in commands {
        if command.command.command_type != EngineCommandType::ReplacePlan {
            forward_commands.push(command.clone());
            continue;
        }

        let command_result = apply_command_with_dedupe(
            &mut deduper,
            &mut stream,
            wall_clock_timestamp_rfc3339(),
            command,
        );
        events.push(command_result.event_record);
        if !command_result.accepted || command_result.duplicate {
            continue;
        }

        let new_plan = decode_replace_plan(command)?;
        let diff = diff_plans_json(active_plan, &new_plan);
        if let Some(reason_code) =
            forbidden_replace_reason(active_plan, &new_plan, &state.completed_node_ids)
        {
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                replace_plan_error_event(&command.command.id, reason_code, &diff),
            ));
            state.paused_reason = Some(format!("replace_plan_rejected:{}", reason_code.as_str()));
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                engine_paused_event("replace_plan_rejected"),
            ));
            pause_after_processing = true;
            continue;
        }

        let requires_confirm = diff.summary.removed > 0 || diff.summary.changed > 0;
        let confirmed = command
            .command
            .data
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if requires_confirm && !confirmed {
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                replace_plan_confirm_event(&command.command.id, &diff),
            ));
            state.paused_reason = Some("need_user_confirm:replace_plan".to_string());
            events.push(stream.next_record(
                wall_clock_timestamp_rfc3339(),
                engine_paused_event("need_user_confirm"),
            ));
            pause_after_processing = true;
            continue;
        }

        build_router_executor_for_plan(&new_plan, config)
            .map_err(RunnerError::ConfigInvalidForPlan)?;
        let before_hash = active_plan_hash.clone();
        *active_plan = new_plan;
        *active_plan_hash = hash_plan(active_plan)?;
        if state.plan_hash_history.last() != Some(active_plan_hash) {
            state.plan_hash_history.push(active_plan_hash.clone());
        }
        state.plan_epoch = state.plan_epoch.saturating_add(1);
        plan_replaced = true;

        events.push(stream.next_record(
            wall_clock_timestamp_rfc3339(),
            plan_replaced_event(
                &command.command.id,
                command.command.data.get("reason").and_then(Value::as_str),
                &before_hash,
                active_plan_hash.as_str(),
                state.plan_epoch,
                &diff,
            ),
        ));
    }

    state.seen_command_ids = deduper.seen_command_ids();
    state.next_seq = stream.next_seq();

    Ok(ReplacePlanCommandProcessing {
        forward_commands,
        events,
        plan_replaced,
        pause_after_processing,
    })
}

fn decode_replace_plan(
    command: &EngineCommandEnvelope,
) -> Result<ais_sdk::PlanDocument, RunnerError> {
    let plan_value = command.command.data.get("plan").cloned().ok_or_else(|| {
        RunnerError::ReplacePlanInvalid {
            command_id: command.command.id.clone(),
            reason: "replace_plan.data.plan is required".to_string(),
        }
    })?;
    let plan_text = serde_json::to_string(&plan_value)?;
    let parsed = parse_document_with_options(
        plan_text.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .map_err(|issues| RunnerError::ReplacePlanInvalid {
        command_id: command.command.id.clone(),
        reason: format!("replace_plan schema validation failed: {issues:?}"),
    })?;
    match parsed {
        AisDocument::Plan(plan) => Ok(plan),
        _ => Err(RunnerError::ReplacePlanInvalid {
            command_id: command.command.id.clone(),
            reason: "replace_plan.data.plan must be AIS plan document".to_string(),
        }),
    }
}

fn forbidden_replace_reason(
    before: &ais_sdk::PlanDocument,
    after: &ais_sdk::PlanDocument,
    completed_node_ids: &[String],
) -> Option<ReplacePlanReasonCode> {
    let before_by_id = before
        .nodes
        .iter()
        .filter_map(|node| {
            let id = node
                .as_object()
                .and_then(|object| object.get("id"))
                .and_then(Value::as_str)?;
            Some((id.to_string(), node))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let after_by_id = after
        .nodes
        .iter()
        .filter_map(|node| {
            let id = node
                .as_object()
                .and_then(|object| object.get("id"))
                .and_then(Value::as_str)?;
            Some((id.to_string(), node))
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    for node_id in completed_node_ids {
        let Some(before_node) = before_by_id.get(node_id) else {
            continue;
        };
        let Some(after_node) = after_by_id.get(node_id) else {
            return Some(ReplacePlanReasonCode::CompletedNodeRemoved);
        };
        if *before_node != *after_node {
            return Some(ReplacePlanReasonCode::CompletedNodeMutated);
        }
    }
    None
}

fn replace_plan_error_event(
    command_id: &str,
    reason_code: ReplacePlanReasonCode,
    diff: &ais_engine::PlanDiffJson,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::Error);
    event.data.insert(
        "reason".to_string(),
        Value::String("replace_plan rejected".to_string()),
    );
    event.data.insert(
        "reason_code".to_string(),
        Value::String(reason_code.as_str().to_string()),
    );
    event
        .data
        .insert("retryable".to_string(), Value::Bool(false));
    event.data.insert(
        "error".to_string(),
        json!({
            "reason_code": reason_code.as_str(),
            "command_id": command_id,
            "diff": diff
        }),
    );
    event
}

fn replace_plan_confirm_event(command_id: &str, diff: &ais_engine::PlanDiffJson) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::NeedUserConfirm);
    event.data.insert(
        "reason_code".to_string(),
        Value::String("replace_plan_confirmation_required".to_string()),
    );
    event.data.insert(
        "reason".to_string(),
        Value::String("replace_plan requires confirmation".to_string()),
    );
    event.data.insert(
        "details".to_string(),
        json!({
            "node_id": "replace_plan",
            "action_ref": "command:replace_plan",
            "hit_reasons": ["replace_plan_high_risk_diff"],
            "command_id": command_id,
            "diff": diff
        }),
    );
    event
}

fn plan_replaced_event(
    command_id: &str,
    reason: Option<&str>,
    before_plan_hash: &str,
    after_plan_hash: &str,
    plan_epoch: u64,
    diff: &ais_engine::PlanDiffJson,
) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::PlanReplaced);
    event.data.insert(
        "before_plan_hash".to_string(),
        Value::String(before_plan_hash.to_string()),
    );
    event.data.insert(
        "after_plan_hash".to_string(),
        Value::String(after_plan_hash.to_string()),
    );
    event.data.insert(
        "command_id".to_string(),
        Value::String(command_id.to_string()),
    );
    event
        .data
        .insert("plan_epoch".to_string(), Value::Number(plan_epoch.into()));
    event.data.insert("diff".to_string(), json!(diff));
    if let Some(reason) = reason {
        event
            .data
            .insert("reason".to_string(), Value::String(reason.to_string()));
    }
    event
}

fn engine_paused_event(reason: &str) -> EngineEvent {
    let mut event = EngineEvent::new(EngineEventType::EnginePaused);
    event
        .data
        .insert("reason_code".to_string(), Value::String(reason.to_string()));
    event
        .data
        .insert("reason".to_string(), Value::String(reason.to_string()));
    event
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplacePlanReasonCode {
    CompletedNodeRemoved,
    CompletedNodeMutated,
}

impl ReplacePlanReasonCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::CompletedNodeRemoved => "replace_plan_completed_node_removed",
            Self::CompletedNodeMutated => "replace_plan_completed_node_mutated",
        }
    }
}

fn evaluate_workflow_outputs(
    workflow: &ais_sdk::WorkflowDocument,
    runtime: &Value,
) -> Result<Value, RunnerError> {
    let context = ResolverContext::with_runtime(runtime.clone());
    let mut out = serde_json::Map::new();
    for (key, value_ref_json) in &workflow.outputs {
        let value_ref: ValueRef =
            serde_json::from_value(value_ref_json.clone()).map_err(|error| {
                RunnerError::WorkflowOutputs(format!("`{key}` invalid ValueRef: {error}"))
            })?;
        let value =
            evaluate_value_ref_with_options(&value_ref, &context, &ValueRefEvalOptions::default())
                .map_err(|error| {
                    RunnerError::WorkflowOutputs(format!("`{key}` evaluation failed: {error}"))
                })?;
        out.insert(key.clone(), value);
    }
    Ok(Value::Object(out))
}

fn render_execution_output(
    command: &PlanCommand,
    state: &EngineRunnerState,
    status: EngineRunStatus,
    resumed_from_checkpoint: bool,
    iteration: usize,
    events: &[ais_engine::EngineEventRecord],
) -> Result<String, RunnerError> {
    if command.events_jsonl.as_deref() == Some("-") {
        let mut out = String::new();
        for event in events {
            out.push_str(
                encode_event_jsonl_line(event)
                    .map_err(|error| RunnerError::EventsIo(error.to_string()))?
                    .as_str(),
            );
        }
        return Ok(out);
    }

    let completed_set = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let status_text = match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    };
    let output = match command.format {
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "schema": "ais-runner-run-plan/0.0.1",
            "status": status_text,
            "paused_reason": state.paused_reason,
            "resumed_from_checkpoint": resumed_from_checkpoint,
            "iterations": iteration,
            "events_emitted": events.len(),
            "command_accepted": count_event_type(events, EngineEventType::CommandAccepted),
            "command_rejected": count_event_type(events, EngineEventType::CommandRejected),
            "completed_node_ids": completed_set,
        }))?,
        OutputFormat::Text => format!(
            "AIS run plan\nstatus: {}\npaused_reason: {}\nresumed_from_checkpoint: {}\niterations: {}\nevents: {}\ncommand_accepted: {}\ncommand_rejected: {}\ncompleted_nodes: {}",
            status_text,
            state.paused_reason.clone().unwrap_or_else(|| "none".to_string()),
            resumed_from_checkpoint,
            iteration,
            events.len(),
            count_event_type(events, EngineEventType::CommandAccepted),
            count_event_type(events, EngineEventType::CommandRejected),
            completed_set.into_iter().collect::<Vec<_>>().join(",")
        ),
    };
    Ok(output)
}

fn write_event_sinks(
    command: &PlanCommand,
    events: &[ais_engine::EngineEventRecord],
) -> Result<(), RunnerError> {
    if command.verbose {
        write_verbose_events(events);
    }

    if let Some(target) = &command.events_jsonl {
        if target == "-" {
            return Ok(());
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(target)
            .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
        for event in events {
            let line = encode_event_jsonl_line(event)
                .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
            file.write_all(line.as_bytes())
                .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
        }
    }

    if let Some(path) = &command.trace {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| RunnerError::TraceIo(error.to_string()))?;
        let redact = TraceRedactOptions::default();
        for event in events {
            let line = encode_trace_jsonl_line(event, &redact)
                .map_err(|error| RunnerError::TraceIo(error.to_string()))?;
            file.write_all(line.as_bytes())
                .map_err(|error| RunnerError::TraceIo(error.to_string()))?;
        }
    }

    Ok(())
}

fn write_verbose_events(events: &[EngineEventRecord]) {
    for record in events {
        let event_type = serde_json::to_value(record.event.event_type)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{:?}", record.event.event_type).to_lowercase());
        let node_id = record.event.node_id.as_deref().unwrap_or("-");
        let reason = record
            .event
            .data
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let checks = record
            .event
            .data
            .get("checks")
            .and_then(Value::as_object)
            .map(|checks| {
                let mut parts = Vec::<String>::new();
                for check_key in ["condition", "gate", "assert"] {
                    let Some(result) = checks
                        .get(check_key)
                        .and_then(Value::as_object)
                        .and_then(|item| item.get("result"))
                        .and_then(Value::as_bool)
                    else {
                        continue;
                    };
                    parts.push(format!("{check_key}={result}"));
                }
                if parts.is_empty() {
                    "-".to_string()
                } else {
                    parts.join(",")
                }
            })
            .unwrap_or_else(|| "-".to_string());
        eprintln!(
            "[event seq={} type={} node={} reason={} checks={}]",
            record.seq, event_type, node_id, reason, checks
        );
        if event_type == "error" {
            if let Ok(detail) = serde_json::to_string(&record.event.data) {
                eprintln!("[event detail seq={}] {}", record.seq, detail);
            }
        }
    }
}

fn reconcile_pending_side_effects(
    ledger: &mut RunnerCheckpointLedger,
    router: &ais_engine::RouterExecutor,
    state: &mut EngineRunnerState,
) -> Result<Option<String>, RunnerError> {
    let pending = ledger.pending_side_effects();
    if pending.is_empty() {
        return Ok(None);
    }

    let mut pending_nodes = BTreeSet::<String>::new();
    let mut reverted_nodes = BTreeSet::<String>::new();
    let mut completed_nodes = state
        .completed_node_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    for effect in pending {
        match router.reconcile_side_effect(&effect) {
            Ok(result) => {
                let Some(updated) = result.record else {
                    pending_nodes.insert(effect.node_id.clone());
                    continue;
                };
                let node_id = updated.node_id.clone();
                match updated.status.as_str() {
                    SIDE_EFFECT_STATUS_CONFIRMED => {
                        completed_nodes.insert(node_id.clone());
                    }
                    SIDE_EFFECT_STATUS_REVERTED => {
                        reverted_nodes.insert(node_id.clone());
                    }
                    _ => {
                        pending_nodes.insert(node_id.clone());
                    }
                }
                ledger.upsert_side_effect(updated);
            }
            Err(_) => {
                pending_nodes.insert(effect.node_id.clone());
            }
        }
    }

    state.completed_node_ids = completed_nodes.into_iter().collect();

    if !reverted_nodes.is_empty() {
        return Ok(Some(format!(
            "side_effect_reconcile_reverted:{}",
            reverted_nodes.into_iter().collect::<Vec<_>>().join(",")
        )));
    }
    if !pending_nodes.is_empty() {
        return Ok(Some(format!(
            "side_effect_reconcile_pending:{}",
            pending_nodes.into_iter().collect::<Vec<_>>().join(",")
        )));
    }

    Ok(None)
}

fn record_side_effect_lifecycle(runtime: &mut Value, ledger: &RunnerCheckpointLedger) {
    if !runtime.is_object() {
        *runtime = Value::Object(serde_json::Map::new());
    }
    let Some(root) = runtime.as_object_mut() else {
        return;
    };
    let agent_entry = root
        .entry("agent".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !agent_entry.is_object() {
        *agent_entry = Value::Object(serde_json::Map::new());
    }
    if let Some(agent) = agent_entry.as_object_mut() {
        agent.insert(
            "side_effect_lifecycle".to_string(),
            ledger.side_effect_lifecycle_summary(),
        );
    }
}

fn maybe_save_checkpoint(
    command: &PlanCommand,
    run_id: &str,
    plan_hash: &str,
    plan: &ais_sdk::PlanDocument,
    state: &EngineRunnerState,
    ledger: &RunnerCheckpointLedger,
) -> Result<(), RunnerError> {
    let Some(path) = &command.checkpoint else {
        return Ok(());
    };
    let mut checkpoint = create_checkpoint_document(
        run_id.to_string(),
        plan_hash.to_string(),
        CheckpointEngineState {
            completed_node_ids: state.completed_node_ids.clone(),
            paused_reason: state.paused_reason.clone(),
            seen_command_ids: state.seen_command_ids.clone(),
            pending_retries: state.pending_retries.clone(),
            plan_epoch: state.plan_epoch,
            plan_hash_history: state.plan_hash_history.clone(),
        },
        Some(state.runtime.clone()),
        Some(serde_json::to_value(plan)?),
        None,
    );
    checkpoint.approvals_ledger = ledger.approvals();
    checkpoint.side_effects = ledger.side_effects();
    save_checkpoint_to_path(path, &checkpoint).map_err(|error| RunnerError::CheckpointSave {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    Ok(())
}

fn hash_plan(plan: &ais_sdk::PlanDocument) -> Result<String, RunnerError> {
    let bytes = serde_json::to_vec(plan)?;
    let digest = Sha256::digest(bytes);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

fn read_plan_document(
    path: &Path,
    parse_error: impl Fn(String) -> RunnerError,
) -> Result<ais_sdk::PlanDocument, RunnerError> {
    let text = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let parsed = parse_document_with_options(
        text.as_str(),
        ParseDocumentOptions {
            format: DocumentFormat::Auto,
            validate_schema: true,
        },
    )
    .map_err(|issues| parse_error(format!("{issues:?}")))?;

    match parsed {
        AisDocument::Plan(plan) => Ok(plan),
        _ => Err(parse_error(
            "input file must be AIS plan document".to_string(),
        )),
    }
}

fn replay_status_label(status: ais_engine::ReplayStatus) -> &'static str {
    match status {
        ais_engine::ReplayStatus::Completed => "completed",
        ais_engine::ReplayStatus::Paused => "paused",
        ais_engine::ReplayStatus::ReachedUntilNode => "reached_until_node",
    }
}

fn render_replay_output(
    command: &ReplayCommand,
    events: &[EngineEventRecord],
    status: &str,
    completed_node_ids: &[String],
    paused_reason: Option<&str>,
) -> Result<String, RunnerError> {
    match command.format {
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "schema": "ais-runner-replay/0.0.1",
            "status": status,
            "events_emitted": events.len(),
            "completed_node_ids": completed_node_ids,
            "paused_reason": paused_reason,
        }))
        .map_err(RunnerError::from),
        OutputFormat::Text => Ok(format!(
            "AIS replay\nstatus: {status}\nevents: {}\ncompleted_nodes: {}\npaused_reason: {}",
            events.len(),
            completed_node_ids.join(","),
            paused_reason.unwrap_or("none")
        )),
    }
}

fn count_event_type(
    events: &[ais_engine::EngineEventRecord],
    event_type: EngineEventType,
) -> usize {
    events
        .iter()
        .filter(|record| record.event.event_type == event_type)
        .count()
}

fn read_commands_from_stdin(enabled: bool) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
    if !enabled {
        return Ok(Vec::new());
    }
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    read_command_jsonl(reader)
}

fn read_command_jsonl(reader: impl BufRead) -> Result<Vec<EngineCommandEnvelope>, RunnerError> {
    let mut commands = Vec::<EngineCommandEnvelope>::new();
    for (line_index, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|error| RunnerError::EventsIo(error.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let envelope = decode_command_jsonl_line(line.as_str()).map_err(|error| {
            RunnerError::CommandDecode {
                line: line_index + 1,
                reason: error.to_string(),
            }
        })?;
        commands.push(envelope);
    }
    Ok(commands)
}

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
