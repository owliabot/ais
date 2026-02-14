use crate::cli::{OutputFormat, PlanCommand, PlanDiffCommand, ReplayCommand, WorkflowCommand};
use crate::config::{build_router_executor_for_plan, load_runner_config};
use crate::io::load_workspace_documents_excluding;
use ais_core::StructuredIssue;
use ais_engine::{
    create_checkpoint_document, decode_command_jsonl_line, encode_event_jsonl_line,
    encode_trace_jsonl_line, load_checkpoint_from_path, run_plan_once, save_checkpoint_to_path,
    CheckpointEngineState, DefaultSolver, EngineCommandEnvelope, EngineEventType, diff_plans_json,
    diff_plans_text, replay_from_checkpoint, replay_trace_jsonl, EngineEventRecord, ReplayOptions,
    EngineRunnerOptions, EngineRunnerState, EngineRunStatus, TraceRedactOptions,
};
use ais_sdk::{
    compile_workflow, dry_run_json, dry_run_text, parse_document_with_options,
    validate_workflow_document, validate_workspace_references, AisDocument, CompileWorkflowOptions,
    CompileWorkflowResult, DocumentFormat, ParseDocumentOptions, ResolverContext, ValueRef,
    ValueRefEvalOptions, WorkspaceDocuments, evaluate_value_ref_with_options,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("read file failed `{path}`: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("plan parse failed: {0}")]
    PlanParse(String),
    #[error("before plan parse failed: {0}")]
    PlanDiffBeforeParse(String),
    #[error("after plan parse failed: {0}")]
    PlanDiffAfterParse(String),
    #[error("plan file must be AIS plan document")]
    NotPlanDocument,
    #[error("runtime parse failed: {0}")]
    RuntimeParse(String),
    #[error("runner config path is required for plan execution: pass `--config <file>`")]
    MissingRunnerConfig,
    #[error("runner config load failed: {0}")]
    ConfigLoad(String),
    #[error("runner config invalid for plan: {0:?}")]
    ConfigInvalidForPlan(Vec<StructuredIssue>),
    #[error("replay requires `--trace-jsonl <file>` or `--checkpoint <file>`")]
    ReplayInputRequired,
    #[error("replay from checkpoint requires `--plan <file>`")]
    ReplayMissingPlan,
    #[error("replay from checkpoint requires `--config <file>`")]
    ReplayMissingConfig,
    #[error("replay plan parse failed: {0}")]
    ReplayPlanParse(String),
    #[error("replay trace read failed `{path}`: {source}")]
    ReplayTraceRead {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("replay trace decode failed: {0}")]
    ReplayTraceDecode(String),
    #[error("checkpoint load failed `{path}`: {reason}")]
    CheckpointLoad { path: String, reason: String },
    #[error("checkpoint save failed `{path}`: {reason}")]
    CheckpointSave { path: String, reason: String },
    #[error("write file failed `{path}`: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("write events JSONL failed: {0}")]
    EventsIo(String),
    #[error("write trace JSONL failed: {0}")]
    TraceIo(String),
    #[error("commands stdin jsonl decode failed at line {line}: {reason}")]
    CommandDecode { line: usize, reason: String },
    #[error("engine run reached iteration limit ({0})")]
    IterationLimitExceeded(usize),
    #[error("json encode failed: {0}")]
    JsonEncode(#[from] serde_json::Error),
    #[error("workflow parse failed: {0}")]
    WorkflowParse(String),
    #[error("workspace load failed: {0}")]
    WorkspaceLoad(String),
    #[error("workspace validation failed: {0}")]
    WorkspaceValidate(String),
    #[error("workflow validation failed: {0}")]
    WorkflowValidate(String),
    #[error("workflow compile failed: {0}")]
    WorkflowCompile(String),
    #[error("workflow outputs evaluation failed: {0}")]
    WorkflowOutputs(String),
    #[error("{command} is not implemented yet")]
    NotImplemented { command: &'static str },
}

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
            let runtime_text = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
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
    let workflow_text = fs::read_to_string(&command.workflow).map_err(|source| RunnerError::ReadFile {
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
        _ => return Err(RunnerError::WorkflowParse("workflow file must be AIS workflow document".to_string())),
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
            return Err(RunnerError::WorkspaceValidate(format!("{workspace_issues:?}")));
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
            let runtime_text = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
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
    let before = read_plan_document(
        command.before.as_path(),
        RunnerError::PlanDiffBeforeParse,
    )?;
    let after = read_plan_document(
        command.after.as_path(),
        RunnerError::PlanDiffAfterParse,
    )?;

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
        let trace = fs::read_to_string(trace_path).map_err(|source| RunnerError::ReplayTraceRead {
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
        let plan_path = command.plan.as_ref().ok_or(RunnerError::ReplayMissingPlan)?;
        let config_path = command.config.as_ref().ok_or(RunnerError::ReplayMissingConfig)?;
        let plan = read_plan_document(
            plan_path.as_path(),
            RunnerError::ReplayPlanParse,
        )?;
        let config = load_runner_config(config_path.as_path())
            .map_err(|error| RunnerError::ConfigLoad(error.to_string()))?;
        let router = build_router_executor_for_plan(&plan, &config)
            .map_err(RunnerError::ConfigInvalidForPlan)?;
        let checkpoint = load_checkpoint_from_path(checkpoint_path).map_err(|error| RunnerError::CheckpointLoad {
            path: checkpoint_path.display().to_string(),
            reason: error.to_string(),
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
    let router = build_router_executor_for_plan(plan, &config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    let mut queued_commands = Some(read_commands_from_stdin(command.commands_stdin_jsonl)?);

    let plan_hash = hash_plan(plan)?;
    let mut resumed_from_checkpoint = false;
    let mut state = if let Some(checkpoint_path) = &command.checkpoint {
        let checkpoint_usable = fs::metadata(checkpoint_path)
            .map(|meta| meta.is_file() && meta.len() > 0)
            .unwrap_or(false);
        if !checkpoint_usable {
            EngineRunnerState {
                runtime,
                ..EngineRunnerState::default()
            }
        } else {
            match load_checkpoint_from_path(checkpoint_path) {
            Ok(checkpoint) => {
                resumed_from_checkpoint = true;
                if checkpoint.plan_hash != plan_hash {
                    return Err(RunnerError::CheckpointLoad {
                        path: checkpoint_path.display().to_string(),
                        reason: "checkpoint plan hash mismatch".to_string(),
                    });
                }
                EngineRunnerState {
                    runtime: checkpoint.runtime_snapshot.unwrap_or_else(|| runtime.clone()),
                    completed_node_ids: checkpoint.engine_state.completed_node_ids,
                    approved_node_ids: Vec::new(),
                    seen_command_ids: checkpoint.engine_state.seen_command_ids,
                    paused_reason: checkpoint.engine_state.paused_reason,
                    pending_retries: checkpoint.engine_state.pending_retries,
                    next_seq: 0,
                }
            }
            Err(ais_engine::CheckpointStoreError::Io(_)) => EngineRunnerState {
                runtime,
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
            ..EngineRunnerState::default()
        }
    };

    let mut all_events = Vec::<ais_engine::EngineEventRecord>::new();
    let run_id = format!("run-{}", plan_hash.get(0..12).unwrap_or(plan_hash.as_str()));
    let mut iteration = 0usize;
    let max_iterations = plan.nodes.len().saturating_mul(4).max(8);
    let options = EngineRunnerOptions::default();
    let final_status = loop {
        iteration += 1;
        if iteration > max_iterations {
            return Err(RunnerError::IterationLimitExceeded(max_iterations));
        }

        let current_commands = queued_commands.as_deref().unwrap_or(&[]);
        let run_result = run_plan_once(
            run_id.as_str(),
            plan,
            &mut state,
            &router,
            &DefaultSolver,
            current_commands,
            &options,
        );
        queued_commands = None;
        write_event_sinks(command, &run_result.events)?;
        all_events.extend(run_result.events);
        maybe_save_checkpoint(command, run_id.as_str(), &plan_hash, &state)?;

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

fn evaluate_workflow_outputs(
    workflow: &ais_sdk::WorkflowDocument,
    runtime: &Value,
) -> Result<Value, RunnerError> {
    let context = ResolverContext::with_runtime(runtime.clone());
    let mut out = serde_json::Map::new();
    for (key, value_ref_json) in &workflow.outputs {
        let value_ref: ValueRef = serde_json::from_value(value_ref_json.clone())
            .map_err(|error| RunnerError::WorkflowOutputs(format!("`{key}` invalid ValueRef: {error}")))?;
        let value = evaluate_value_ref_with_options(&value_ref, &context, &ValueRefEvalOptions::default())
            .map_err(|error| RunnerError::WorkflowOutputs(format!("`{key}` evaluation failed: {error}")))?;
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
        eprintln!(
            "[event seq={} type={} node={} reason={}]",
            record.seq, event_type, node_id, reason
        );
        if event_type == "error" {
            if let Ok(detail) = serde_json::to_string(&record.event.data) {
                eprintln!("[event detail seq={}] {}", record.seq, detail);
            }
        }
    }
}

fn maybe_save_checkpoint(
    command: &PlanCommand,
    run_id: &str,
    plan_hash: &str,
    state: &EngineRunnerState,
) -> Result<(), RunnerError> {
    let Some(path) = &command.checkpoint else {
        return Ok(());
    };
    let checkpoint = create_checkpoint_document(
        run_id.to_string(),
        plan_hash.to_string(),
        CheckpointEngineState {
            completed_node_ids: state.completed_node_ids.clone(),
            paused_reason: state.paused_reason.clone(),
            seen_command_ids: state.seen_command_ids.clone(),
            pending_retries: state.pending_retries.clone(),
        },
        Some(state.runtime.clone()),
        None,
    );
    save_checkpoint_to_path(path, &checkpoint).map_err(|error| RunnerError::CheckpointSave {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    Ok(())
}

fn hash_plan(plan: &ais_sdk::PlanDocument) -> Result<String, RunnerError> {
    let bytes = serde_json::to_vec(plan)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
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
        _ => Err(parse_error("input file must be AIS plan document".to_string())),
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

fn count_event_type(events: &[ais_engine::EngineEventRecord], event_type: EngineEventType) -> usize {
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
        let envelope = decode_command_jsonl_line(line.as_str()).map_err(|error| RunnerError::CommandDecode {
            line: line_index + 1,
            reason: error.to_string(),
        })?;
        commands.push(envelope);
    }
    Ok(commands)
}

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
