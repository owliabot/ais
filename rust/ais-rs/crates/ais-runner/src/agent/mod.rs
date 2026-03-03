mod brain;
mod budget;
mod candidates;
mod checkpoint_ext;
mod checkpoint_flow;
mod context;
mod context_view;
mod error_state;
mod input_normalize;
mod input_store;
mod intent_context;
mod intent_segmented;
mod r#loop;
mod missing_input;
mod orchestrator;
mod phase_machine;
mod planning_memory;
mod prompts;
mod runtime_store;
mod sanitize;
mod summary;
mod todos;
mod tools;
mod trace;
mod write_gates;

use crate::checkpoint_ledger::RunnerCheckpointLedger;
use crate::cli::{AgentCommand, AgentProfile, OutputFormat};
use crate::config::{
    build_router_executor_for_plan, load_runner_config, RunnerConfig, SignerConfig,
};
use crate::error::RunnerError;
use crate::policy::{
    approvals_mode_from_pack, llm_may_approve_max_risk_level_from_pack, load_pack_document,
    policy_from_pack,
};
use crate::run::process_replace_plan_commands;
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_engine::events::wall_clock_timestamp_rfc3339;
use ais_engine::{
    create_checkpoint_document, encode_event_jsonl_line, encode_trace_jsonl_line,
    load_checkpoint_from_path, save_checkpoint_to_path, CheckpointEngineState, DefaultSolver,
    EngineCommandEnvelope, EngineCommandType, EngineEventRecord, EngineRunStatus,
    EngineRunnerOptions, EngineRunnerState, TraceRedactOptions, SIDE_EFFECT_STATUS_CONFIRMED,
    SIDE_EFFECT_STATUS_REVERTED,
};
use ais_evm_executor::LocalPrivateKeySigner as EvmLocalPrivateKeySigner;
use ais_llm::providers::{
    build_provider, build_provider_chain, ProviderChainConfig, ProviderChainPolicy,
    ProviderConfig as LlmProviderConfig, RotationMode,
};
use ais_llm::{CompleteWithToolsResponse, LlmProvider, ScriptedLlmProvider};
use ais_sdk::documents::{
    PlanSketchCatalogSnapshot, PlanSketchDocument, PlanSketchPackSnapshot, PlanSketchSegment,
    PlanSketchSession,
};
use ais_sdk::{
    compile_plan_sketch, parse_document_with_options, AisDocument, CompilePlanSketchOptions,
    CompilePlanSketchResult, DocumentFormat, ParseDocumentOptions, PlanDocument, ResolverContext,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

use brain::AgentDecisionPolicy;
pub use brain::LlmBrain;
use budget::compact_json_for_llm;
use candidates::{
    build_candidate_context_for_agent, CandidateContext, DEFAULT_MAX_INDEX_CANDIDATES,
};
use input_store::{
    InputStore, InputStoreUpsertResult, InputValueLayer, InputValueMeta, InputValueStability,
};
use intent_segmented::{
    IntentGroundingDraft, IntentGroundingRequest, LlmSegmentedIntentPlanner, SegmentBeginRequest,
    SegmentDraft, SegmentPlanningRequest, SegmentedIntentPlanner, SegmentedPromptOverrides,
    TodoDraft, TodoPlanningRequest,
};
use prompts::PromptCatalog;
use r#loop::{run_agent_loop, AgentLoopConfig, CommandBuilder};
use todos::TodoBoard;

#[derive(Debug, Clone, Deserialize)]
struct MissingInputOptionPrompt {
    #[serde(default)]
    value: Option<Value>,
    label: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MissingInputQuestionPrompt {
    id: String,
    question: String,
    #[serde(default)]
    options: Vec<MissingInputOptionPrompt>,
    #[serde(default)]
    required: Option<bool>,
}

const AIS_INPUT_STORE_MIGRATION_MODE_ENV: &str = "AIS_RUNNER_INPUT_STORE_MIGRATION_MODE";

/// Input source migration toggle used during P3 rollout.
#[derive(Debug, Clone, Copy)]
pub(super) enum InputStoreMigrationMode {
    /// Historical behavior: use legacy runtime.inputs reads/writes.
    Legacy,
    /// New writes go to InputStore first, but keep runtime.inputs as writable mirror.
    ShadowWrites,
    /// Read path prefers InputStore and falls back to runtime projection.
    ReadThrough,
    /// Single source: InputStore is canonical and runtime.inputs is projection-only.
    SingleSource,
    /// Hard failure on legacy-only behavior to ensure no mixed sources in production.
    EnforcedSingleSource,
}

impl InputStoreMigrationMode {
    fn from_env() -> Self {
        let raw = match env::var(AIS_INPUT_STORE_MIGRATION_MODE_ENV) {
            Ok(value) => value,
            Err(_) => return Self::SingleSource,
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "legacy" => Self::Legacy,
            "shadow" | "shadow_writes" | "writes" | "shadowwrites" => Self::ShadowWrites,
            "read" | "readthrough" | "read_through" => Self::ReadThrough,
            "single" | "single_source" | "single-source" => Self::SingleSource,
            "enforce" | "strict" | "enforced" | "enforced_single_source" => {
                Self::EnforcedSingleSource
            }
            _ => Self::SingleSource,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::ShadowWrites => "shadow_writes",
            Self::ReadThrough => "read_through",
            Self::SingleSource => "single_source",
            Self::EnforcedSingleSource => "enforced_single_source",
        }
    }
}

fn report_input_store_migration_mode_if_verbose(command: &AgentCommand) {
    if !command.verbose {
        return;
    }
    let mode = InputStoreMigrationMode::from_env();
    eprintln!(
        "[agent][input_store] migration_mode={} input_read_source=InputStoreOnly runtime_inputs_role=projection_only",
        mode.name(),
    );
}

fn upsert_input_value(
    runtime: &mut Value,
    key: &str,
    value: Value,
    source: impl Into<String>,
    source_priority: u32,
    provenance: impl Into<String>,
) -> InputStoreUpsertResult {
    let Some(canonical_key) = input_normalize::normalize_input_slot_key(key) else {
        return InputStoreUpsertResult::Rejected;
    };

    let source = source.into();
    let provenance = provenance.into();
    let mut input_store = InputStore::default();
    let result = if source_priority >= 100 || source == "user" {
        input_store.upsert_user(canonical_key.as_str(), value, provenance)
    } else {
        input_store.upsert_seed(canonical_key.as_str(), value, provenance)
    };
    if matches!(
        result,
        InputStoreUpsertResult::Inserted | InputStoreUpsertResult::Replaced
    ) && input_store.has(canonical_key.as_str())
    {
        if let Some(entry) = input_store.get(canonical_key.as_str()) {
            input_normalize::set_runtime_input_value(
                runtime,
                canonical_key.as_str(),
                entry.value.clone(),
            );
        }
    }
    result
}

fn upsert_seed_input_value(
    runtime: &mut Value,
    key: &str,
    value: Value,
    provenance: impl Into<String>,
) -> InputStoreUpsertResult {
    upsert_input_value(runtime, key, value, "seed", 10, provenance)
}

fn upsert_user_input_value(
    runtime: &mut Value,
    key: &str,
    value: Value,
    provenance: impl Into<String>,
) -> InputStoreUpsertResult {
    upsert_input_value(runtime, key, value, "user", 100, provenance)
}

pub(super) fn upsert_store_value_with_source(
    store: &mut InputStore,
    key: impl AsRef<str>,
    value: Value,
    layer: InputValueLayer,
    source: &str,
    source_priority: u32,
    provenance: impl Into<String>,
) -> InputStoreUpsertResult {
    let key_ref = key.as_ref();
    let stability = if key_ref.to_ascii_lowercase().contains("decimal") {
        InputValueStability::Stable
    } else if source.eq_ignore_ascii_case("query")
        && (key_ref.to_ascii_lowercase().contains("balance")
            || key_ref.to_ascii_lowercase().contains("allowance"))
    {
        InputValueStability::Volatile
    } else {
        InputValueStability::Unknown
    };
    let observed_at_ms = source.eq_ignore_ascii_case("query").then_some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    );
    store.upsert(
        key_ref,
        value,
        InputValueMeta {
            source: source.to_string(),
            source_priority,
            provenance: Some(provenance.into()),
            confidence: None,
            layer,
            stability,
            observed_at_ms,
        },
    )
}

pub fn execute_agent(command: &AgentCommand) -> Result<String, RunnerError> {
    validate_agent_profile(command)?;
    report_input_store_migration_mode_if_verbose(command);
    let config = load_runner_config(command.config.as_path())
        .map_err(|error| RunnerError::ConfigLoad(error.to_string()))?;
    let pack = match &command.pack {
        Some(path) => Some(load_pack_document(path.as_path())?),
        None => None,
    };
    let max_index_candidates = command
        .max_index_candidates
        .unwrap_or(DEFAULT_MAX_INDEX_CANDIDATES);
    let prompt_catalog = PromptCatalog::from_prompts_dir(
        config
            .llm
            .as_ref()
            .and_then(|llm| llm.prompts_dir.as_deref()),
    );
    let candidate_context =
        build_candidate_context_for_agent(command, pack.as_ref(), max_index_candidates)?;
    if command.plan.is_none() {
        return execute_segmented_intent_agent(
            command,
            &config,
            pack.as_ref(),
            candidate_context.clone(),
            &prompt_catalog,
        );
    }
    let mut active_plan = read_plan_document(
        command
            .plan
            .as_ref()
            .ok_or_else(|| RunnerError::Llm("missing --plan".to_string()))?
            .as_path(),
    )?;
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
    let mut active_plan_hash = hash_plan(&active_plan)?;
    let run_id = format!(
        "run-{}",
        active_plan_hash
            .get(0..12)
            .unwrap_or(active_plan_hash.as_str())
    );
    let (
        mut state,
        resumed_from_checkpoint,
        checkpoint_plan,
        checkpoint_plan_hash,
        mut checkpoint_ledger,
        _checkpoint_extensions,
    ) = load_or_init_state(command, &active_plan_hash, runtime)?;
    if let Some(plan) = checkpoint_plan {
        active_plan = plan;
        active_plan_hash = checkpoint_plan_hash.unwrap_or(hash_plan(&active_plan)?);
    }
    let router = build_router_executor_for_plan(&active_plan, &config)
        .map_err(RunnerError::ConfigInvalidForPlan)?;
    if resumed_from_checkpoint {
        record_side_effect_lifecycle(&mut state.runtime, &checkpoint_ledger);
        if let Some(paused_reason) =
            reconcile_pending_side_effects(&mut checkpoint_ledger, &router, &mut state)
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
                None,
            )?;
            return render_agent_output(
                command,
                &state,
                EngineRunStatus::Paused,
                0,
                0,
                resumed_from_checkpoint,
            );
        }
    }

    let mut total_events = 0usize;
    let derived_mode = command
        .approvals_mode
        .or_else(|| pack.as_ref().and_then(approvals_mode_from_pack))
        .unwrap_or(crate::cli::ApprovalsMode::Safe);
    let assist_threshold = if derived_mode == crate::cli::ApprovalsMode::Assist {
        pack.as_ref()
            .and_then(llm_may_approve_max_risk_level_from_pack)
    } else {
        None
    };
    let mut decision_policy = build_decision_policy(
        command,
        &config,
        derived_mode,
        assist_threshold,
        candidate_context,
        &prompt_catalog,
    )?;

    let max_iterations = command
        .max_iterations
        .unwrap_or_else(|| active_plan.nodes.len().saturating_mul(8).max(16));
    let loop_config = AgentLoopConfig { max_iterations };
    let mut engine_options = EngineRunnerOptions::default();
    if let Some(pack) = &pack {
        engine_options.policy = policy_from_pack(pack)
            .map_err(|error| RunnerError::WorkspaceValidate(error.to_string()))?;
    }
    let mut command_builder = CommandBuilder::new(run_id.as_str());
    let mut total_iterations = 0usize;
    let final_status = loop {
        let loop_result = run_agent_loop(
            run_id.as_str(),
            &active_plan,
            &mut state,
            &router,
            &DefaultSolver,
            &engine_options,
            &loop_config,
            &mut command_builder,
            &mut decision_policy,
            |state, events| {
                total_events += events.len();
                write_event_sinks(command, events)?;
                checkpoint_ledger.absorb_events(events);
                checkpoint_ledger.mark_approved_nodes(
                    &state.approved_node_ids,
                    wall_clock_timestamp_rfc3339().as_str(),
                );
                maybe_save_checkpoint(
                    command,
                    run_id.as_str(),
                    &active_plan_hash,
                    &active_plan,
                    state,
                    &checkpoint_ledger,
                    None,
                )?;
                Ok(())
            },
        )?;
        total_iterations += loop_result.iterations;
        match loop_result.status {
            EngineRunStatus::Completed | EngineRunStatus::Stopped => {
                break loop_result.status;
            }
            EngineRunStatus::Paused => break EngineRunStatus::Paused,
        }
    };

    Ok(render_agent_output(
        command,
        &state,
        final_status,
        total_iterations,
        total_events,
        resumed_from_checkpoint,
    )?)
}

fn execute_segmented_intent_agent(
    command: &AgentCommand,
    config: &RunnerConfig,
    pack: Option<&ais_sdk::PackDocument>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
) -> Result<String, RunnerError> {
    orchestrator::execute_segmented_intent_agent(
        command,
        config,
        pack,
        candidate_context,
        prompt_catalog,
    )
}

fn empty_plan_document() -> PlanDocument {
    PlanDocument {
        schema: "ais-plan/0.0.3".to_string(),
        meta: Some(Value::Object(serde_json::Map::new())),
        nodes: vec![],
        extensions: serde_json::Map::new(),
    }
}

fn merge_segment_plan(
    base: &PlanDocument,
    segment: &PlanDocument,
) -> Result<PlanDocument, RunnerError> {
    let mut merged = base.clone();
    let replacement_segment_ids = segment
        .nodes
        .iter()
        .filter_map(plan_sketch_segment_id)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if replacement_segment_ids.is_empty() {
        merged.nodes.extend(segment.nodes.clone());
    } else {
        merged.nodes.retain(|node| {
            plan_sketch_segment_id(node)
                .map(|segment_id| !replacement_segment_ids.contains(segment_id))
                .unwrap_or(true)
        });
        merged.nodes.extend(segment.nodes.clone());
    }
    if merged.meta.is_none() {
        merged.meta = Some(Value::Object(serde_json::Map::new()));
    }
    if let Some(meta) = merged.meta.as_mut().and_then(Value::as_object_mut) {
        let next_segment_count = meta
            .get("extensions")
            .and_then(Value::as_object)
            .and_then(|extensions| extensions.get("segment_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .saturating_add(1);
        let extensions = meta
            .entry("extensions".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !extensions.is_object() {
            *extensions = Value::Object(serde_json::Map::new());
        }
        if let Some(extensions_object) = extensions.as_object_mut() {
            extensions_object.insert(
                "segment_count".to_string(),
                Value::Number(next_segment_count.into()),
            );
        }
    }
    Ok(merged)
}

fn plan_sketch_segment_id(node: &Value) -> Option<&str> {
    node.pointer("/extensions/plan_sketch/segment_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
fn compile_segment_plan(
    intent: &str,
    session: &intent_segmented::SegmentPlanningSession,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    pack: Option<&ais_sdk::PackDocument>,
    chain_scope: &[String],
) -> Result<PlanDocument, Value> {
    compile_segment_plan_with_inputs(
        intent,
        session,
        segment,
        candidate_context,
        pack,
        chain_scope,
        &[],
    )
}

fn compile_segment_plan_with_inputs(
    intent: &str,
    session: &intent_segmented::SegmentPlanningSession,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    pack: Option<&ais_sdk::PackDocument>,
    chain_scope: &[String],
    known_input_refs: &[String],
) -> Result<PlanDocument, Value> {
    let pack_snapshot_hash = derive_pack_snapshot_hash(pack).map_err(|error| {
        serde_json::json!({
            "reason_code": "snapshot_hash_error",
            "message": error.to_string(),
        })
    })?;
    compile_segment_plan_with_snapshot_hash_and_inputs(
        intent,
        session.session_id.as_str(),
        session.cursor.as_str(),
        segment,
        candidate_context,
        pack_snapshot_hash.as_str(),
        chain_scope,
        known_input_refs,
    )
}

pub(super) fn compile_segment_plan_with_snapshot_hash(
    intent: &str,
    session_id: &str,
    cursor: &str,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    pack_snapshot_hash: &str,
    chain_scope: &[String],
    known_input_refs: &[String],
) -> Result<PlanDocument, Value> {
    compile_segment_plan_with_snapshot_hash_and_inputs(
        intent,
        session_id,
        cursor,
        segment,
        candidate_context,
        pack_snapshot_hash,
        chain_scope,
        known_input_refs,
    )
}

fn compile_segment_plan_with_snapshot_hash_and_inputs(
    intent: &str,
    session_id: &str,
    cursor: &str,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    pack_snapshot_hash: &str,
    chain_scope: &[String],
    known_input_refs: &[String],
) -> Result<PlanDocument, Value> {
    let normalized_segment = normalize_segment_asset_inputs_for_compile(
        segment,
        candidate_context,
        chain_scope.first().map(String::as_str),
        known_input_refs,
    );
    validate_segment_write_gates(&normalized_segment, candidate_context, None)?;

    let mut resolver = ResolverContext::new();
    for protocol in &candidate_context.protocols {
        resolver.register_protocol(protocol.clone());
    }

    let sketch = PlanSketchDocument {
        schema: "ais-plan-sketch/0.1.0".to_string(),
        intent: intent.to_string(),
        pack_snapshot: PlanSketchPackSnapshot {
            name: None,
            version: None,
            hash: pack_snapshot_hash.to_string(),
        },
        catalog_snapshot: PlanSketchCatalogSnapshot {
            schema: candidate_context
                .executable_candidates
                .catalog_schema
                .clone(),
            hash: candidate_context.executable_candidates.catalog_hash.clone(),
        },
        chain_scope: chain_scope.to_vec(),
        session: Some(PlanSketchSession {
            session_id: session_id.to_string(),
            cursor: cursor.to_string(),
        }),
        segments: vec![normalized_segment],
        meta: None,
        extensions: serde_json::Map::new(),
    };

    match compile_plan_sketch(
        &sketch,
        &resolver,
        Some(&candidate_context.executable_candidates),
        &CompilePlanSketchOptions {
            default_chain: chain_scope.first().cloned(),
            known_input_refs: build_known_input_refs(known_input_refs),
        },
    ) {
        CompilePlanSketchResult::Ok { plan } => Ok(plan),
        CompilePlanSketchResult::Err { issues } => Err(serde_json::json!({
            "reason_code": "compile_error",
            "message": "segment compile failed",
            "issues": issues,
        })),
    }
}

fn normalize_segment_asset_inputs_for_compile(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    default_chain: Option<&str>,
    known_input_refs: &[String],
) -> PlanSketchSegment {
    let mut out = segment.clone();
    let known_chain_ref = known_input_refs
        .iter()
        .filter_map(|raw| input_normalize::normalize_input_slot_key(raw))
        .map(|slot| format!("inputs.{slot}"))
        .find(|slot| {
            slot == "inputs.chain" || slot == "inputs.chain_id" || slot == "inputs.chain_ref"
        });
    let chain_binding = if let Some(chain_ref) = known_chain_ref {
        serde_json::json!({"ref": chain_ref})
    } else {
        serde_json::json!({"lit": default_chain.unwrap_or("eip155:1")})
    };

    for step in &mut out.steps {
        if step.kind != "query" && step.kind != "action" {
            continue;
        }
        let Some(candidate_ref) = step.candidate_ref.as_deref() else {
            continue;
        };
        let Some(detail) = candidate_context.detail_by_ref.get(candidate_ref) else {
            continue;
        };
        let Some(params) = detail.get("params").and_then(Value::as_array) else {
            continue;
        };
        for param in params {
            let Some(param_name) = param.get("name").and_then(Value::as_str) else {
                continue;
            };
            let is_asset = param
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("asset"));
            if !is_asset {
                continue;
            }
            let Some(existing) = step.inputs.get(param_name).cloned() else {
                continue;
            };
            if asset_input_already_structured(&existing) {
                continue;
            }
            step.inputs.insert(
                param_name.to_string(),
                serde_json::json!({
                    "object": {
                        "address": existing,
                        "chain_ref": chain_binding
                    }
                }),
            );
        }
    }

    out
}

fn asset_input_already_structured(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if let Some(asset_object) = object.get("object").and_then(Value::as_object) {
        return asset_object.get("address").is_some();
    }
    object.get("address").is_some()
}

#[cfg(test)]
fn compile_segment_plan_with_snapshot_hash_and_facts(
    intent: &str,
    session_id: &str,
    cursor: &str,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    pack_snapshot_hash: &str,
    chain_scope: &[String],
    input_store: Option<&InputStore>,
) -> Result<PlanDocument, Value> {
    let known_input_refs = input_store
        .map(collect_known_input_refs_from_input_store_semantics)
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    compile_segment_plan_with_snapshot_hash_and_inputs(
        intent,
        session_id,
        cursor,
        segment,
        candidate_context,
        pack_snapshot_hash,
        chain_scope,
        known_input_refs.as_slice(),
    )
}

fn build_known_input_refs(known_input_refs: &[String]) -> Vec<String> {
    known_input_refs
        .iter()
        .filter_map(|raw_ref| {
            input_normalize::normalize_input_slot_key(raw_ref)
                .map(|canonical_slot| format!("inputs.{canonical_slot}"))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

pub(super) fn known_input_refs_from_state_summary(state_summary: Option<&Value>) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for raw_ref in state_summary
        .and_then(|summary| summary.pointer("/input_registry/known_refs"))
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
    {
        if let Some(canonical_slot) = input_normalize::normalize_input_slot_key(raw_ref) {
            refs.insert(format!("inputs.{canonical_slot}"));
        }
    }
    for raw_ref in state_summary
        .and_then(|summary| summary.pointer("/input_slots/canonical_refs"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|entries| entries.values())
        .filter_map(Value::as_str)
    {
        if let Some(canonical_slot) = input_normalize::normalize_input_slot_key(raw_ref) {
            refs.insert(format!("inputs.{canonical_slot}"));
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn grounding_fact_keys_from_state_summary(state_summary: Option<&Value>) -> Vec<String> {
    let mut keys = BTreeSet::<String>::new();
    for raw_key in intent_context::grounding_fact_keys_from_state_summary(state_summary) {
        if let Some(normalized) = normalize_fact_hint_key(raw_key.as_str()) {
            keys.insert(normalized);
        }
    }
    keys.into_iter().collect::<Vec<_>>()
}

pub(super) fn canonicalize_segment_input_refs(
    segment: &PlanSketchSegment,
    known_refs: &[String],
    grounding_fact_keys: &[String],
) -> Result<PlanSketchSegment, Value> {
    let allowed_refs = known_refs
        .iter()
        .filter_map(|raw_ref| {
            input_normalize::normalize_input_slot_key(raw_ref)
                .map(|canonical_slot| format!("inputs.{canonical_slot}"))
        })
        .collect::<BTreeSet<_>>();
    let grounding_fact_keys = grounding_fact_keys
        .iter()
        .filter_map(|key| normalize_fact_hint_key(key))
        .collect::<BTreeSet<_>>();
    let mut value = serde_json::to_value(segment).map_err(|error| {
        serde_json::json!({
            "reason_code": "compile_error",
            "message": format!("segment input-ref guard failed to encode segment: {error}"),
            "issues": []
        })
    })?;
    let mut issues = Vec::<Value>::new();
    canonicalize_segment_input_refs_value(
        &mut value,
        "",
        &allowed_refs,
        &grounding_fact_keys,
        &mut issues,
    );
    if !issues.is_empty() {
        return Err(serde_json::json!({
            "reason_code": "compile_error",
            "message": "segment compile blocked by input ref guard",
            "issues": issues,
        }));
    }
    serde_json::from_value(value).map_err(|error| {
        serde_json::json!({
            "reason_code": "compile_error",
            "message": format!("segment input-ref guard failed to decode segment: {error}"),
            "issues": []
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TodoExecutionScope {
    QueryOnly,
    Mixed,
    WriteOnly,
}

pub(super) fn validate_segment_todo_scope(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    current_todo: Option<&Value>,
) -> Result<(), Value> {
    let scope = infer_todo_execution_scope(current_todo);
    if scope != TodoExecutionScope::QueryOnly {
        return Ok(());
    }
    let mut issues = Vec::<Value>::new();
    for step in &segment.steps {
        let is_write_step = if step.kind == "action" {
            true
        } else if let Some(candidate_ref) = step.candidate_ref.as_deref() {
            candidate_context
                .detail_by_ref
                .get(candidate_ref)
                .is_some_and(candidate_detail_is_write_action)
        } else {
            false
        };
        if !is_write_step {
            continue;
        }
        issues.push(serde_json::json!({
            "kind":"todo_scope_violation",
            "reason_code":"todo_scope_violation",
            "message":"current todo scope is query_only; write/action steps are not allowed in this segment",
            "step_id": step.id,
            "candidate_ref": step.candidate_ref,
            "segment_id": segment.segment_id,
        }));
    }
    if issues.is_empty() {
        return Ok(());
    }
    Err(serde_json::json!({
        "reason_code":"todo_scope_violation",
        "message":"segment violates current todo scope",
        "issues": issues
    }))
}

fn candidate_detail_is_write_action(detail: &Value) -> bool {
    detail.get("kind").and_then(Value::as_str) == Some("action")
}

fn infer_todo_execution_scope(current_todo: Option<&Value>) -> TodoExecutionScope {
    let Some(todo) = current_todo.and_then(Value::as_object) else {
        return TodoExecutionScope::Mixed;
    };
    if let Some(explicit) = todo.get("execution_scope").and_then(Value::as_str) {
        match explicit.trim().to_ascii_lowercase().as_str() {
            "query_only" => return TodoExecutionScope::QueryOnly,
            "write_only" => return TodoExecutionScope::WriteOnly,
            "mixed" => return TodoExecutionScope::Mixed,
            _ => {}
        }
    }
    let mut corpus = Vec::<String>::new();
    if let Some(title) = todo.get("title").and_then(Value::as_str) {
        corpus.push(title.to_ascii_lowercase());
    }
    for field in ["acceptance", "required_facts", "produced_facts"] {
        for raw in todo
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flat_map(|items| items.iter())
            .filter_map(Value::as_str)
        {
            corpus.push(raw.to_ascii_lowercase());
        }
    }
    let has_write = corpus.iter().any(|entry| {
        [
            "transfer", "swap", "approve", "send", "execute", "write", "withdraw", "deposit",
            "mint", "burn",
        ]
        .iter()
        .any(|needle| entry.contains(needle))
    });
    if has_write {
        return TodoExecutionScope::Mixed;
    }
    let has_query = corpus.iter().any(|entry| {
        [
            "balance", "query", "check", "read", "retrieve", "fact", "verify",
        ]
        .iter()
        .any(|needle| entry.contains(needle))
    });
    if has_query {
        return TodoExecutionScope::QueryOnly;
    }
    TodoExecutionScope::Mixed
}

fn canonicalize_segment_input_refs_value(
    value: &mut Value,
    path: &str,
    allowed_refs: &BTreeSet<String>,
    grounding_fact_keys: &BTreeSet<String>,
    issues: &mut Vec<Value>,
) {
    match value {
        Value::Object(map) => {
            if let Some(raw_ref) = map.get("ref").and_then(Value::as_str) {
                let ref_path = append_ref_guard_path(path, "ref");
                if let Some(result) =
                    resolve_canonical_input_ref(raw_ref, allowed_refs, grounding_fact_keys)
                {
                    match result {
                        CanonicalInputRefResolution::Resolved(canonical_ref) => {
                            if canonical_ref != raw_ref {
                                map.insert("ref".to_string(), Value::String(canonical_ref));
                            }
                        }
                        CanonicalInputRefResolution::Unknown {
                            normalized_ref,
                            candidates,
                        } => {
                            issues.push(serde_json::json!({
                                "kind": "validation",
                                "reference": "unknown_input_ref",
                                "path": ref_path,
                                "raw_ref": raw_ref,
                                "normalized_ref": normalized_ref,
                                "suggested_ref": candidates.first().cloned().unwrap_or_else(|| normalized_ref.clone()),
                                "candidates": candidates,
                                "message": format!("unknown input ref `{raw_ref}`; allowed refs must come from state_summary.input_registry.known_refs"),
                                "guard": "input_ref_canonicalization",
                            }));
                        }
                    }
                }
            }
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let child_path = append_ref_guard_path(path, &key);
                if let Some(child) = map.get_mut(&key) {
                    canonicalize_segment_input_refs_value(
                        child,
                        child_path.as_str(),
                        allowed_refs,
                        grounding_fact_keys,
                        issues,
                    );
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let child_path = if path.is_empty() {
                    format!("[{index}]")
                } else {
                    format!("{path}[{index}]")
                };
                canonicalize_segment_input_refs_value(
                    item,
                    child_path.as_str(),
                    allowed_refs,
                    grounding_fact_keys,
                    issues,
                );
            }
        }
        _ => {}
    }
}

enum CanonicalInputRefResolution {
    Resolved(String),
    Unknown {
        normalized_ref: String,
        candidates: Vec<String>,
    },
}

fn resolve_canonical_input_ref(
    raw_ref: &str,
    allowed_refs: &BTreeSet<String>,
    grounding_fact_keys: &BTreeSet<String>,
) -> Option<CanonicalInputRefResolution> {
    if !looks_like_input_ref(raw_ref) {
        return None;
    }
    let normalized_ref = canonicalize_input_ref_alias(raw_ref)
        .unwrap_or_else(|| coarse_input_ref_alias(raw_ref).unwrap_or_else(|| raw_ref.to_string()));
    if allowed_refs.contains(&normalized_ref) {
        return Some(CanonicalInputRefResolution::Resolved(normalized_ref));
    }
    Some(CanonicalInputRefResolution::Unknown {
        normalized_ref: normalized_ref.clone(),
        candidates: ranked_input_ref_candidates(
            raw_ref,
            normalized_ref.as_str(),
            allowed_refs,
            grounding_fact_keys,
            5,
        ),
    })
}

fn looks_like_input_ref(raw_ref: &str) -> bool {
    let trimmed = raw_ref.trim();
    trimmed.starts_with("inputs.")
        || trimmed.starts_with("runtime.inputs.")
        || trimmed.starts_with("input.")
}

fn canonicalize_input_ref_alias(raw_ref: &str) -> Option<String> {
    let trimmed = raw_ref.trim().trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | ')' | '(')
    });
    let without_prefix = if let Some(suffix) = trimmed.strip_prefix("runtime.inputs.") {
        suffix
    } else if let Some(suffix) = trimmed.strip_prefix("inputs.") {
        suffix
    } else if let Some(suffix) = trimmed.strip_prefix("input.") {
        suffix
    } else {
        return None;
    };
    let normalized = without_prefix
        .strip_suffix(".value")
        .unwrap_or(without_prefix)
        .replace(':', ".");
    let canonical_slot = input_normalize::normalize_input_slot_key(normalized.as_str())?;
    Some(format!("inputs.{canonical_slot}"))
}

fn coarse_input_ref_alias(raw_ref: &str) -> Option<String> {
    let trimmed = raw_ref.trim();
    let without_prefix = if let Some(suffix) = trimmed.strip_prefix("runtime.inputs.") {
        suffix
    } else if let Some(suffix) = trimmed.strip_prefix("inputs.") {
        suffix
    } else if let Some(suffix) = trimmed.strip_prefix("input.") {
        suffix
    } else {
        return None;
    };
    let slot = without_prefix
        .strip_suffix(".value")
        .unwrap_or(without_prefix)
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    (!slot.is_empty()).then_some(format!("inputs.{slot}"))
}

fn ranked_input_ref_candidates(
    raw_ref: &str,
    normalized_ref: &str,
    allowed_refs: &BTreeSet<String>,
    grounding_fact_keys: &BTreeSet<String>,
    limit: usize,
) -> Vec<String> {
    let mut ranked = Vec::<String>::new();
    for exact in exact_input_ref_candidates(raw_ref, normalized_ref) {
        if allowed_refs.contains(&exact) {
            ranked.push(exact);
        }
    }
    for alias in deterministic_alias_input_ref_candidates(
        raw_ref,
        normalized_ref,
        allowed_refs,
        grounding_fact_keys,
        limit,
    ) {
        if !ranked.contains(&alias) {
            ranked.push(alias);
        }
    }
    for candidate in semantic_input_ref_candidates(normalized_ref, allowed_refs, limit) {
        if !ranked.contains(&candidate) {
            ranked.push(candidate);
        }
    }
    ranked.truncate(limit.max(1));
    ranked
}

fn exact_input_ref_candidates(raw_ref: &str, normalized_ref: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for candidate in [
        Some(normalized_ref.to_string()),
        canonicalize_input_ref_alias(raw_ref),
        coarse_input_ref_alias(raw_ref),
    ]
    .into_iter()
    .flatten()
    {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn deterministic_alias_input_ref_candidates(
    raw_ref: &str,
    normalized_ref: &str,
    allowed_refs: &BTreeSet<String>,
    grounding_fact_keys: &BTreeSet<String>,
    limit: usize,
) -> Vec<String> {
    let alias_keys = alias_keys_for_unknown_ref(raw_ref, normalized_ref, grounding_fact_keys);
    if alias_keys.is_empty() {
        return Vec::new();
    }

    let mut ranked = allowed_refs
        .iter()
        .filter_map(|candidate| {
            let candidate_slot = candidate
                .strip_prefix("inputs.")
                .unwrap_or(candidate.as_str());
            let score = alias_keys
                .iter()
                .map(|alias_key| {
                    deterministic_alias_score(
                        alias_key,
                        candidate_slot,
                        grounding_fact_keys.contains(alias_key),
                    )
                })
                .max()
                .unwrap_or(0);
            (score > 0).then_some((score, candidate.clone()))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    ranked.truncate(limit.max(1));
    ranked.into_iter().map(|(_, candidate)| candidate).collect()
}

fn alias_keys_for_unknown_ref(
    raw_ref: &str,
    normalized_ref: &str,
    grounding_fact_keys: &BTreeSet<String>,
) -> BTreeSet<String> {
    let slot = normalized_ref
        .strip_prefix("inputs.")
        .unwrap_or(normalized_ref)
        .to_ascii_lowercase();
    let mut keys = BTreeSet::<String>::new();
    if let Some(suffix) = slot.strip_prefix("fact.") {
        if let Some(normalized) = normalize_fact_hint_key(suffix) {
            keys.insert(normalized);
        }
    }
    if let Some(suffix) = slot.strip_prefix("facts.") {
        if let Some(normalized) = normalize_fact_hint_key(suffix) {
            keys.insert(normalized);
        }
    }
    if raw_ref.contains("fact:") || raw_ref.contains("facts:") {
        for marker in ["fact:", "facts:"] {
            if let Some((_, suffix)) = raw_ref.split_once(marker) {
                if let Some(token) = suffix.split(['.', ':', '/', ' ']).next() {
                    if let Some(normalized) = normalize_fact_hint_key(token) {
                        keys.insert(normalized);
                    }
                }
            }
        }
    }
    for fact_key in grounding_fact_keys {
        if slot.contains(fact_key) {
            keys.insert(fact_key.clone());
        }
    }
    keys
}

fn deterministic_alias_score(alias_key: &str, candidate_slot: &str, grounded: bool) -> i32 {
    let candidate_slot = candidate_slot.to_ascii_lowercase();
    let alias_key = alias_key.to_ascii_lowercase();
    let alias_tokens = input_ref_tokens(alias_key.as_str());
    let candidate_tokens = input_ref_tokens(candidate_slot.as_str());
    let mut score = 0;
    if candidate_slot == alias_key {
        score += 600;
    } else if candidate_slot.starts_with(format!("{alias_key}.").as_str()) {
        score += 520;
    } else if candidate_slot.starts_with(format!("{alias_key}_").as_str()) {
        score += 500;
    } else if candidate_slot.ends_with(format!(".{alias_key}").as_str()) {
        score += 420;
    } else if candidate_slot.contains(alias_key.as_str()) {
        score += 260;
    }
    if !alias_tokens.is_empty()
        && alias_tokens
            .iter()
            .all(|token| candidate_tokens.contains(token))
    {
        score += 220;
    }
    if grounded && alias_key == "token" {
        if candidate_tokens.contains(&"address".to_string()) {
            score += 320;
        }
        if candidate_tokens.contains(&"symbol".to_string()) {
            score += 20;
        }
        if candidate_tokens.contains(&"decimals".to_string()) {
            score += 20;
        }
    }
    score
}

fn semantic_input_ref_candidates(
    normalized_ref: &str,
    allowed_refs: &BTreeSet<String>,
    limit: usize,
) -> Vec<String> {
    let target_slot = normalized_ref
        .strip_prefix("inputs.")
        .unwrap_or(normalized_ref);
    let target_tokens = input_ref_tokens(target_slot);
    let target_leaf = target_tokens.last().cloned().unwrap_or_default();
    let mut ranked = allowed_refs
        .iter()
        .filter_map(|candidate| {
            let candidate_slot = candidate
                .strip_prefix("inputs.")
                .unwrap_or(candidate.as_str());
            let candidate_tokens = input_ref_tokens(candidate_slot);
            let candidate_leaf = candidate_tokens.last().cloned().unwrap_or_default();
            let shared_tokens = target_tokens
                .iter()
                .filter(|token| candidate_tokens.contains(token))
                .count() as i32;
            let mut score = shared_tokens * 10;
            if !target_leaf.is_empty() && target_leaf == candidate_leaf {
                score += 12;
            }
            if candidate_slot.starts_with(target_slot) || target_slot.starts_with(candidate_slot) {
                score += 8;
            }
            if candidate_slot.contains(target_slot) || target_slot.contains(candidate_slot) {
                score += 4;
            }
            (score > 0).then_some((score, candidate.clone()))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    ranked.truncate(limit.max(1));
    ranked.into_iter().map(|(_, candidate)| candidate).collect()
}

fn input_ref_tokens(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>()
}

fn normalize_fact_hint_key(raw: &str) -> Option<String> {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace(':', ".")
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    (!normalized.is_empty()).then_some(normalized)
}

fn append_ref_guard_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else if key.starts_with('[') {
        format!("{path}{key}")
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
fn collect_known_input_refs_from_input_store_semantics(
    store: &InputStore,
) -> std::collections::BTreeSet<String> {
    let mut refs = std::collections::BTreeSet::<String>::new();
    for slot in store.list_ref_strings() {
        refs.insert(format!("inputs.{slot}"));
    }
    refs
}

fn validate_segment_write_gates(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    input_store: Option<&InputStore>,
) -> Result<(), Value> {
    write_gates::validate_segment_write_gates(segment, candidate_context, input_store)
}

fn derive_pack_snapshot_hash(pack: Option<&ais_sdk::PackDocument>) -> Result<String, RunnerError> {
    let value = pack
        .map(serde_json::to_value)
        .transpose()?
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    stable_hash_hex(&value, &StableJsonOptions::default())
        .map_err(|error| RunnerError::Llm(format!("compute pack snapshot hash failed: {error}")))
}

fn derive_planning_snapshot_hash(
    pack_snapshot_hash: &str,
    catalog_hash: &str,
    chain_scope: &[String],
    approvals_mode: Option<crate::cli::ApprovalsMode>,
) -> Result<String, RunnerError> {
    stable_hash_hex(
        &serde_json::json!({
            "pack_snapshot_hash": pack_snapshot_hash,
            "catalog_hash": catalog_hash,
            "chain_scope": chain_scope,
            "approvals_mode": approvals_mode.map(|mode| format!("{mode:?}")).unwrap_or_else(|| "safe".to_string())
        }),
        &StableJsonOptions::default(),
    )
    .map_err(|error| RunnerError::Llm(format!("compute planning snapshot hash failed: {error}")))
}

fn derive_chain_scope(candidate_context: &CandidateContext) -> Vec<String> {
    let plugins = candidate_context
        .index_candidates
        .get("execution_plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::<String>::new();
    for plugin in plugins {
        let Some(chain) = plugin.get("chain").and_then(Value::as_str) else {
            continue;
        };
        if chain.is_empty() || out.iter().any(|value| value == chain) {
            continue;
        }
        out.push(chain.to_string());
    }
    if out.is_empty() {
        out.push("eip155:1".to_string());
    }
    out
}

fn resolve_planner_context_token_budget(command: &AgentCommand, config: &RunnerConfig) -> usize {
    command
        .planner_context_token_budget
        .or_else(|| {
            config
                .llm
                .as_ref()
                .and_then(|llm| llm.planner_context_token_budget)
        })
        .unwrap_or(context_view::DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET)
        .max(1)
}

fn resolve_segmented_max_tool_rounds(command: &AgentCommand, config: &RunnerConfig) -> u8 {
    command
        .max_tool_rounds
        .or_else(|| config.llm.as_ref().and_then(|llm| llm.max_tool_rounds))
        .unwrap_or(intent_segmented::DEFAULT_SEGMENTED_MAX_TOOL_ROUNDS)
        .max(1)
}

fn resolve_llm_context_limit_tokens(config: &RunnerConfig) -> Option<usize> {
    config
        .llm
        .as_ref()
        .and_then(|llm| llm.context_limit_tokens)
        .filter(|value| *value > 0)
}

fn build_initial_input_store(
    runtime: &Value,
    config: &RunnerConfig,
    chain_scope: &[String],
) -> Result<InputStore, RunnerError> {
    let mut store = InputStore::default();
    seed_runtime_input_facts(runtime, &mut store);
    seed_runtime_owner_facts(runtime, &mut store);
    seed_signer_owner_facts(config, chain_scope, &mut store)?;
    Ok(store)
}

fn decode_agent_checkpoint_extensions(
    runtime: &mut Value,
    checkpoint_extensions: Option<&Map<String, Value>>,
    verbose_llm: bool,
) -> checkpoint_ext::AgentCheckpointExtensions {
    let extensions = checkpoint_ext::AgentCheckpointExtensions::decode(checkpoint_extensions);
    if let Some(todo_progress) = extensions.todo_progress() {
        record_runtime_agent_field(runtime, "todo_progress", todo_progress.clone());
    }
    if let Some(intent_facts) = extensions.intent_facts() {
        if runtime
            .pointer("/agent/intent_grounding/intent_facts")
            .is_none()
        {
            let mut grounding = runtime
                .pointer("/agent/intent_grounding")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            grounding.insert(
                "intent_facts".to_string(),
                Value::Object(intent_facts.clone().into_iter().collect()),
            );
            record_runtime_agent_field(runtime, "intent_grounding", Value::Object(grounding));
        }
    }
    if verbose_llm {
        eprintln!(
            "[checkpoint] input_store projection restored={}",
            extensions.input_store().is_some()
        );
        eprintln!(
            "[checkpoint] todo_progress restored={}",
            extensions.todo_progress().is_some()
        );
        eprintln!(
            "[checkpoint] intent_facts restored={}",
            extensions.intent_facts().is_some()
        );
    }
    extensions
}

fn seed_runtime_owner_facts(runtime: &Value, store: &mut InputStore) {
    let mut candidates = Vec::<(String, String)>::new();
    for key in ["inputs.owner", "inputs.wallet"] {
        let Some(entry) = store.get(key) else {
            continue;
        };
        let Some(owner) = entry.value.as_str().map(str::trim) else {
            continue;
        };
        if owner.is_empty() {
            continue;
        }
        candidates.push((
            entry
                .meta
                .provenance
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            owner.to_string(),
        ));
    }
    for (provenance, value) in [
        (
            "runtime.ctx.wallet_address".to_string(),
            runtime
                .pointer("/ctx/wallet_address")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
        (
            "runtime.owner".to_string(),
            runtime
                .pointer("/owner")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
        (
            "runtime.wallet".to_string(),
            runtime
                .pointer("/wallet")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
    ] {
        if let Some(owner) = value {
            candidates.push((provenance, owner));
        }
    }
    for (provenance, owner) in candidates {
        let owner_value = Value::String(owner);
        upsert_store_value_with_source(
            store,
            "owner",
            owner_value.clone(),
            InputValueLayer::Seed,
            "runtime",
            70,
            provenance.clone(),
        );
        upsert_store_value_with_source(
            store,
            "wallet.default",
            owner_value,
            InputValueLayer::Seed,
            "runtime",
            70,
            provenance,
        );
    }
}

fn seed_runtime_input_facts(runtime: &Value, store: &mut InputStore) {
    let Some(inputs) = runtime.pointer("/inputs") else {
        return;
    };
    let mut path = Vec::<String>::new();
    seed_runtime_input_facts_recursive(inputs, &mut path, store);
}

fn seed_runtime_input_facts_recursive(
    value: &Value,
    path: &mut Vec<String>,
    store: &mut InputStore,
) {
    if !path.is_empty() {
        let raw_slot = path.join(".");
        if let Some(slot) = input_normalize::normalize_input_slot_key(raw_slot.as_str()) {
            let provenance = format!("runtime.inputs.{slot}");
            let _ = upsert_store_value_with_source(
                store,
                slot.as_str(),
                value.clone(),
                InputValueLayer::Seed,
                "runtime",
                70,
                provenance,
            );
        }
    }
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let Some(child) = object.get(key.as_str()) else {
                    continue;
                };
                path.push(key);
                seed_runtime_input_facts_recursive(child, path, store);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                seed_runtime_input_facts_recursive(item, path, store);
                path.pop();
            }
        }
        _ => {}
    }
}

fn seed_signer_owner_facts(
    config: &RunnerConfig,
    chain_scope: &[String],
    store: &mut InputStore,
) -> Result<(), RunnerError> {
    let mut chains = BTreeSet::<String>::new();
    if chain_scope.is_empty() {
        chains.extend(config.chains.keys().cloned());
    } else {
        chains.extend(chain_scope.iter().cloned());
    }

    for chain in chains {
        let Some(chain_config) = config.chains.get(chain.as_str()) else {
            continue;
        };
        let Some(signer) = chain_config.signer.as_ref() else {
            continue;
        };
        let SignerConfig::EvmPrivateKey { private_key } = signer else {
            continue;
        };

        let signer = EvmLocalPrivateKeySigner::from_hex(private_key.as_str()).map_err(|error| {
            RunnerError::Llm(format!(
                "derive owner from signer failed for chain `{chain}`: {error}"
            ))
        })?;
        let owner = format!("{:#x}", signer.address());
        let source = format!("runner_config.chains.{chain}.signer");
        let owner_value = Value::String(owner.clone());
        upsert_store_value_with_source(
            store,
            format!("owner_by_chain.{chain}"),
            owner_value.clone(),
            InputValueLayer::Seed,
            "config",
            80,
            source.clone(),
        );
        upsert_store_value_with_source(
            store,
            "owner",
            owner_value.clone(),
            InputValueLayer::Seed,
            "config",
            80,
            source.clone(),
        );
        upsert_store_value_with_source(
            store,
            "wallet.default",
            owner_value,
            InputValueLayer::Seed,
            "config",
            80,
            source,
        );
    }

    Ok(())
}

#[cfg(test)]
fn build_state_summary(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
) -> Value {
    context_view::build_projected_summary(
        state,
        completed_segments,
        done,
        previous_error,
        input_store,
        None,
    )
}

fn should_attempt_intent_repair(paused_reason: Option<&str>) -> bool {
    error_state::should_attempt_intent_repair(paused_reason)
}

fn should_retry_segmented_planner_output(error: &RunnerError) -> bool {
    error_state::should_retry_segmented_planner_output(error)
}

fn segmented_planner_output_error_payload(
    error: &RunnerError,
    expected_finalize_tool: &str,
    round: u8,
    retry: u8,
    last_failed_finalize: Option<Value>,
) -> Value {
    error_state::segmented_planner_output_error_payload(
        error,
        expected_finalize_tool,
        round,
        retry,
        last_failed_finalize,
    )
}

fn intent_execution_error_payload(
    paused_reason: Option<&str>,
    events: &[EngineEventRecord],
    round: u8,
) -> Value {
    error_state::intent_execution_error_payload(paused_reason, events, round)
}

fn compile_error_state_payload(error_payload: &Value, round: u8) -> Value {
    error_state::compile_error_state_payload(error_payload, round)
}

fn grounding_phase_error_payload(
    reason_code: &str,
    message: Option<&str>,
    issues: &[Value],
    questions: &[Value],
    round: u8,
) -> Value {
    error_state::grounding_phase_error_payload(reason_code, message, issues, questions, round)
}

fn todo_phase_error_payload(
    reason_code: &str,
    message: Option<&str>,
    issues: &[Value],
    questions: &[Value],
    round: u8,
) -> Value {
    error_state::todo_phase_error_payload(reason_code, message, issues, questions, round)
}

#[cfg(test)]
fn missing_required_input_payload(
    message: Option<&str>,
    questions: &[Value],
    issues: &[Value],
    round: u8,
) -> Value {
    missing_input::payload(message, questions, issues, round)
}

#[cfg(test)]
fn record_missing_required_input(runtime: &mut Value, payload: &Value) {
    runtime_store::record_missing_required_input(runtime, payload);
}

#[cfg(test)]
fn record_todo_progress(runtime: &mut Value, todo_board: &TodoBoard) {
    runtime_store::record_todo_progress(runtime, todo_board);
}

fn record_runtime_agent_field(runtime: &mut Value, key: &str, value: Value) {
    runtime_store::record_runtime_agent_field(runtime, key, value);
}

fn maybe_collect_missing_input_answers(
    questions: &[Value],
) -> Result<Option<Map<String, Value>>, RunnerError> {
    if questions.is_empty() {
        return Ok(None);
    }
    let parsed_questions = parse_missing_input_questions(questions);
    if parsed_questions.is_empty() {
        return Ok(None);
    }
    let mut answers = Map::<String, Value>::new();
    let mut pending_questions = Vec::<MissingInputQuestionPrompt>::new();
    for question in parsed_questions {
        if let Some(value) = auto_answer_missing_input_question(&question) {
            eprintln!(
                "[agent][missing_input] {}: auto-select query option",
                question.id
            );
            answers.insert(question.id.clone(), value);
            continue;
        }
        pending_questions.push(question);
    }
    if pending_questions.is_empty() {
        return Ok(Some(answers));
    }
    if !should_prompt_for_missing_input() {
        return if answers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(answers))
        };
    }
    eprintln!(
        "[agent] planner requires additional inputs ({} question(s))",
        pending_questions.len()
    );
    for question in pending_questions {
        if let Some(value) = prompt_missing_input_question(&question)? {
            answers.insert(question.id, value);
        }
    }
    if answers.is_empty() {
        return Ok(None);
    }
    Ok(Some(answers))
}

fn should_prompt_for_missing_input() -> bool {
    #[cfg(test)]
    {
        false
    }
    #[cfg(not(test))]
    {
        use std::io::IsTerminal as _;
        std::io::stdin().is_terminal()
    }
}

fn parse_missing_input_questions(questions: &[Value]) -> Vec<MissingInputQuestionPrompt> {
    questions
        .iter()
        .filter_map(|item| serde_json::from_value::<MissingInputQuestionPrompt>(item.clone()).ok())
        .collect::<Vec<_>>()
}

fn auto_answer_missing_input_question(question: &MissingInputQuestionPrompt) -> Option<Value> {
    let query_options = question
        .options
        .iter()
        .filter(|option| missing_input_option_is_query(option))
        .collect::<Vec<_>>();
    if query_options.len() != 1 {
        return None;
    }
    let selected = query_options[0];
    Some(
        selected
            .value
            .clone()
            .unwrap_or_else(|| Value::String(selected.label.clone())),
    )
}

fn missing_input_option_is_query(option: &MissingInputOptionPrompt) -> bool {
    if query_hint(option.label.as_str()) {
        return true;
    }
    option.value.as_ref().is_some_and(value_has_query_hint)
}

fn value_has_query_hint(value: &Value) -> bool {
    match value {
        Value::String(raw) => query_hint(raw),
        Value::Object(object) => object.values().any(value_has_query_hint),
        Value::Array(items) => items.iter().any(value_has_query_hint),
        _ => false,
    }
}

fn query_hint(raw: &str) -> bool {
    raw.trim().to_ascii_lowercase().contains("query")
}

fn prompt_missing_input_question(
    question: &MissingInputQuestionPrompt,
) -> Result<Option<Value>, RunnerError> {
    let required = question.required.unwrap_or(true);
    loop {
        eprintln!(
            "[agent][missing_input] {}: {}",
            question.id, question.question
        );
        if !question.options.is_empty() {
            for (index, option) in question.options.iter().enumerate() {
                if let Some(description) = option.description.as_deref() {
                    eprintln!("  {}. {} ({description})", index + 1, option.label);
                } else {
                    eprintln!("  {}. {}", index + 1, option.label);
                }
            }
        }
        if question.options.is_empty() {
            eprint!("[agent][missing_input] enter value");
        } else {
            eprint!(
                "[agent][missing_input] choose 1-{} or enter custom value",
                question.options.len()
            );
        }
        if !required {
            eprint!(" (or `skip`)");
        }
        eprint!(": ");
        std::io::stderr()
            .flush()
            .map_err(|error| RunnerError::EventsIo(error.to_string()))?;

        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
        let input = line.trim();
        if input.is_empty() {
            if required {
                eprintln!("[agent] input is required; please provide a value");
                continue;
            }
            return Ok(None);
        }
        if !required && matches!(input, "skip" | "s") {
            return Ok(None);
        }
        if let Ok(index) = input.parse::<usize>() {
            if index >= 1 && index <= question.options.len() {
                let selected = &question.options[index - 1];
                return Ok(Some(
                    selected
                        .value
                        .clone()
                        .unwrap_or_else(|| Value::String(selected.label.clone())),
                ));
            }
        }
        return Ok(Some(parse_user_supplied_answer_value(input)));
    }
}

fn parse_user_supplied_answer_value(input: &str) -> Value {
    serde_json::from_str::<Value>(input).unwrap_or_else(|_| Value::String(input.to_string()))
}

#[cfg(test)]
fn apply_missing_input_answers(
    state: &mut EngineRunnerState,
    input_store: &mut InputStore,
    answers: &Map<String, Value>,
) {
    missing_input::apply_answers(state, input_store, answers);
}

fn build_replace_plan_command(
    builder: &mut CommandBuilder,
    plan: &ais_sdk::PlanDocument,
    paused_reason: Option<&str>,
) -> Result<EngineCommandEnvelope, RunnerError> {
    let mut normalized_plan = plan.clone();
    if normalized_plan.meta.is_none() {
        normalized_plan.meta = Some(Value::Object(serde_json::Map::new()));
    }
    let mut data = serde_json::Map::new();
    data.insert("plan".to_string(), serde_json::to_value(normalized_plan)?);
    data.insert("confirmed".to_string(), Value::Bool(true));
    data.insert(
        "reason".to_string(),
        Value::String(
            paused_reason
                .map(|value| format!("intent_repair:{value}"))
                .unwrap_or_else(|| "intent_repair".to_string()),
        ),
    );
    Ok(builder.envelope(EngineCommandType::ReplacePlan, data))
}

fn resolve_intent_text(command: &AgentCommand) -> Result<String, RunnerError> {
    if let Some(intent) = command.intent.as_ref() {
        let intent = intent.trim();
        if intent.is_empty() {
            return Err(RunnerError::Llm("`--intent` must be non-empty".to_string()));
        }
        return Ok(intent.to_string());
    }
    if let Some(path) = command.intent_file.as_ref() {
        let input = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
        let intent = input.trim();
        if intent.is_empty() {
            return Err(RunnerError::Llm(format!(
                "`--intent-file {}` must be non-empty",
                path.display()
            )));
        }
        return Ok(intent.to_string());
    }
    Err(RunnerError::Llm(
        "agent requires one of `--plan`, `--intent`, or `--intent-file`".to_string(),
    ))
}

fn validate_agent_profile(command: &AgentCommand) -> Result<(), RunnerError> {
    match (command.profile, command.llm_script_jsonl.as_ref()) {
        (AgentProfile::Standard, Some(_)) => Err(RunnerError::AgentProfile(
            "profile=standard forbids `--llm-script-jsonl`; use `--profile demo-scripted`"
                .to_string(),
        )),
        (AgentProfile::DemoScripted, None) => Err(RunnerError::AgentProfile(
            "profile=demo-scripted requires `--llm-script-jsonl <file>`".to_string(),
        )),
        _ => Ok(()),
    }
}

fn load_segmented_prompt_overrides(prompt_catalog: &PromptCatalog) -> SegmentedPromptOverrides {
    SegmentedPromptOverrides {
        base_rules: prompt_catalog.load_lines_prompt("segmented.base_rules"),
        phase_rules_begin: prompt_catalog.load_lines_prompt("segmented.phase.begin"),
        phase_rules_grounding: prompt_catalog.load_lines_prompt("segmented.phase.grounding"),
        phase_rules_todos: prompt_catalog.load_lines_prompt("segmented.phase.todos"),
        phase_rules_propose: prompt_catalog.load_lines_prompt("segmented.phase.propose"),
        phase_rules_revise: prompt_catalog.load_lines_prompt("segmented.phase.revise"),
        contracts_summary: prompt_catalog.load_lines_prompt("segmented.contracts_summary"),
        begin_payload_patch: prompt_catalog.load_json_prompt("segmented.begin.patch"),
        grounding_payload_patch: prompt_catalog.load_json_prompt("segmented.grounding.patch"),
        todos_payload_patch: prompt_catalog.load_json_prompt("segmented.todos.patch"),
        segment_payload_patch: prompt_catalog.load_json_prompt("segmented.segment.patch"),
    }
}

fn build_decision_policy(
    command: &AgentCommand,
    config: &RunnerConfig,
    mode: crate::cli::ApprovalsMode,
    assist_threshold: Option<u8>,
    candidate_context: Option<CandidateContext>,
    prompt_catalog: &PromptCatalog,
) -> Result<AgentDecisionPolicy<Box<dyn LlmProvider>>, RunnerError> {
    let provider = load_llm_provider(command, config)?;
    let llm = provider.map(|provider| {
        let mut llm = brain::LlmBrain::new(provider);
        if let Some(prompt) = prompt_catalog.load_prompt("agent.controller.system") {
            llm = llm.with_system_prompt(prompt);
        }
        if let Some(context) = candidate_context.clone() {
            llm = llm.with_candidate_context(context);
        }
        llm
    });

    let assist_threshold = if mode == crate::cli::ApprovalsMode::Assist {
        assist_threshold
    } else {
        None
    };

    Ok(AgentDecisionPolicy::new(mode, assist_threshold, llm))
}

fn load_llm_provider(
    command: &AgentCommand,
    config: &RunnerConfig,
) -> Result<Option<Box<dyn LlmProvider>>, RunnerError> {
    if let Some(path) = command.llm_script_jsonl.as_ref() {
        let provider = load_scripted_llm_provider(path)?;
        return Ok(Some(Box::new(provider)));
    }
    let Some(llm_config) = config.llm.as_ref() else {
        return Ok(None);
    };
    let provider = if llm_config.fallback.is_empty()
        && llm_config.max_retries_per_provider.unwrap_or(1) <= 1
        && llm_config.rotation == crate::config::RunnerLlmRotationMode::StickyPrimary
    {
        build_provider(LlmProviderConfig {
            provider: llm_config.provider.clone(),
            model: llm_config.model.clone(),
            api_key: llm_config.api_key.clone(),
            api_base: llm_config.api_base.clone(),
        })
        .map_err(|error| RunnerError::Llm(error.to_string()))?
    } else {
        let mut providers = vec![LlmProviderConfig {
            provider: llm_config.provider.clone(),
            model: llm_config.model.clone(),
            api_key: llm_config.api_key.clone(),
            api_base: llm_config.api_base.clone(),
        }];
        providers.extend(
            llm_config
                .fallback
                .iter()
                .map(|fallback| LlmProviderConfig {
                    provider: fallback.provider.clone(),
                    model: fallback.model.clone(),
                    api_key: fallback.api_key.clone(),
                    api_base: fallback.api_base.clone(),
                }),
        );

        build_provider_chain(ProviderChainConfig {
            providers,
            policy: ProviderChainPolicy {
                max_retries_per_provider: llm_config.max_retries_per_provider.unwrap_or(1),
                rotation_mode: match llm_config.rotation {
                    crate::config::RunnerLlmRotationMode::StickyPrimary => {
                        RotationMode::StickyPrimary
                    }
                    crate::config::RunnerLlmRotationMode::RoundRobin => RotationMode::RoundRobin,
                },
            },
        })
        .map_err(|error| RunnerError::Llm(error.to_string()))?
    };
    Ok(Some(provider))
}

fn load_scripted_llm_provider(path: &Path) -> Result<ScriptedLlmProvider, RunnerError> {
    let input = fs::read_to_string(path).map_err(|source| RunnerError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let mut responses = Vec::<Result<CompleteWithToolsResponse, ais_llm::LlmProviderError>>::new();
    for (index, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response: CompleteWithToolsResponse = serde_json::from_str(line).map_err(|error| {
            RunnerError::Llm(format!(
                "llm script decode failed at line {}: {error}",
                index + 1
            ))
        })?;
        responses.push(Ok(response));
    }
    if responses.is_empty() {
        return Err(RunnerError::Llm(
            "llm script must contain at least one json line".to_string(),
        ));
    }
    Ok(ScriptedLlmProvider::from_responses(responses))
}

fn read_plan_document(path: &Path) -> Result<ais_sdk::PlanDocument, RunnerError> {
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
    .map_err(|issues| RunnerError::PlanParse(format!("{issues:?}")))?;

    match parsed {
        AisDocument::Plan(plan) => Ok(plan),
        _ => Err(RunnerError::PlanParse(
            "input file must be AIS plan document".to_string(),
        )),
    }
}

fn parse_runtime_value(input: &str) -> Result<Value, RunnerError> {
    if input.trim_start().starts_with('{') || input.trim_start().starts_with('[') {
        serde_json::from_str::<Value>(input)
            .map_err(|error| RunnerError::RuntimeParse(error.to_string()))
    } else {
        serde_yaml::from_str::<Value>(input)
            .map_err(|error| RunnerError::RuntimeParse(error.to_string()))
    }
}

fn hash_plan(plan: &ais_sdk::PlanDocument) -> Result<String, RunnerError> {
    let bytes = serde_json::to_vec(plan)?;
    let digest = Sha256::digest(bytes);
    Ok(digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>())
}

fn load_or_init_state(
    command: &AgentCommand,
    plan_hash: &str,
    runtime: Value,
) -> Result<
    (
        EngineRunnerState,
        bool,
        Option<ais_sdk::PlanDocument>,
        Option<String>,
        RunnerCheckpointLedger,
        Option<Map<String, Value>>,
    ),
    RunnerError,
> {
    let Some(checkpoint_path) = &command.checkpoint else {
        if command.verbose {
            eprintln!("[checkpoint] disabled (no --checkpoint)");
        }
        return Ok((
            EngineRunnerState {
                runtime,
                plan_hash_history: vec![plan_hash.to_string()],
                ..EngineRunnerState::default()
            },
            false,
            None,
            None,
            RunnerCheckpointLedger::default(),
            None,
        ));
    };

    let checkpoint_usable = fs::metadata(checkpoint_path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false);
    if !checkpoint_usable {
        if command.verbose {
            eprintln!(
                "[checkpoint] no usable checkpoint at {}; start fresh plan_hash={}",
                checkpoint_path.display(),
                plan_hash
            );
        }
        return Ok((
            EngineRunnerState {
                runtime,
                plan_hash_history: vec![plan_hash.to_string()],
                ..EngineRunnerState::default()
            },
            false,
            None,
            None,
            RunnerCheckpointLedger::default(),
            None,
        ));
    }

    match load_checkpoint_from_path(checkpoint_path) {
        Ok(checkpoint) => {
            if let Some(legacy_refs) = collect_legacy_checkpoint_node_refs(&checkpoint) {
                return Err(RunnerError::CheckpointLoad {
                    path: checkpoint_path.display().to_string(),
                    reason: format!(
                        "legacy checkpoint is not supported (found non-canonical node ids with `/`): {}",
                        legacy_refs.join(",")
                    ),
                });
            }
            if command.verbose {
                eprintln!(
                    "[checkpoint] loaded path={} plan_hash={} epoch={}",
                    checkpoint_path.display(),
                    checkpoint.plan_hash,
                    checkpoint.engine_state.plan_epoch
                );
            }
            let mut checkpoint_plan = None;
            if checkpoint.plan_hash != plan_hash {
                let Some(plan_snapshot) = checkpoint.plan_snapshot.clone() else {
                    return Err(RunnerError::CheckpointLoad {
                        path: checkpoint_path.display().to_string(),
                        reason: "checkpoint plan hash mismatch".to_string(),
                    });
                };
                let mut decoded_plan = serde_json::from_value(plan_snapshot).map_err(|error| {
                    RunnerError::CheckpointLoad {
                        path: checkpoint_path.display().to_string(),
                        reason: format!("checkpoint plan snapshot decode failed: {error}"),
                    }
                })?;
                let removed = dedupe_plan_nodes_by_id(&mut decoded_plan);
                if command.verbose && removed > 0 {
                    eprintln!("[checkpoint] deduped plan snapshot nodes removed={removed}");
                }
                checkpoint_plan = Some(decoded_plan);
                if command.verbose {
                    eprintln!(
                        "[checkpoint] restoring plan snapshot due to hash mismatch current={} checkpoint={}",
                        plan_hash, checkpoint.plan_hash
                    );
                }
            }
            let mut active_runtime = checkpoint
                .runtime_snapshot
                .unwrap_or_else(|| runtime.clone());
            if !active_runtime.is_object() {
                active_runtime = runtime.clone();
            }
            let plan_hash_history = if checkpoint.engine_state.plan_hash_history.is_empty() {
                vec![checkpoint.plan_hash.clone()]
            } else {
                checkpoint.engine_state.plan_hash_history
            };
            let checkpoint_ledger = RunnerCheckpointLedger::from_checkpoint(
                &checkpoint.approvals_ledger,
                &checkpoint.side_effects,
            );
            let mut completed_node_ids = checkpoint.engine_state.completed_node_ids;
            checkpoint_ledger
                .reconcile_completed_from_confirmed_side_effects(&mut completed_node_ids);
            let confirmed_write_reuses = checkpoint_ledger.confirmed_write_reuses();
            if !confirmed_write_reuses.is_empty() {
                let node_ids = confirmed_write_reuses
                    .iter()
                    .map(|entry| entry.node_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let confirmation_hashes = confirmed_write_reuses
                    .iter()
                    .map(|entry| entry.confirmation_hash.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if command.verbose || command.verbose_llm {
                    trace::emit(
                        true,
                        "resume",
                        "side_effect_reused",
                        &[
                            ("count", node_ids.len().to_string()),
                            ("node_ids", node_ids.join(",")),
                            ("confirmation_hashes", confirmation_hashes.join(",")),
                        ],
                    );
                    trace::emit(
                        true,
                        "resume",
                        "resume_skip_confirmed_write",
                        &[
                            ("count", node_ids.len().to_string()),
                            ("node_ids", node_ids.join(",")),
                        ],
                    );
                }
                record_runtime_agent_field(
                    &mut active_runtime,
                    "resume_skip_confirmed_write",
                    serde_json::json!({
                        "schema": "ais-agent-resume-skip/0.0.1",
                        "event": "resume_skip_confirmed_write",
                        "reason": "side_effect_reused",
                        "count": node_ids.len(),
                        "node_ids": node_ids,
                        "confirmation_hashes": confirmation_hashes,
                    }),
                );
            }
            let completed_node_set = completed_node_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<String>>();
            let approved_node_ids = checkpoint_ledger
                .approved_node_ids()
                .into_iter()
                .filter(|node_id| !completed_node_set.contains(node_id))
                .collect::<Vec<_>>();
            Ok((
                EngineRunnerState {
                    runtime: active_runtime,
                    completed_node_ids,
                    approved_node_ids,
                    seen_command_ids: checkpoint.engine_state.seen_command_ids,
                    paused_reason: checkpoint.engine_state.paused_reason,
                    pending_retries: checkpoint.engine_state.pending_retries,
                    plan_epoch: checkpoint.engine_state.plan_epoch,
                    plan_hash_history,
                    next_seq: 0,
                },
                true,
                checkpoint_plan,
                Some(checkpoint.plan_hash),
                checkpoint_ledger,
                (!checkpoint.extensions.is_empty()).then_some(checkpoint.extensions),
            ))
        }
        Err(ais_engine::CheckpointStoreError::Io(_)) => Ok((
            EngineRunnerState {
                runtime,
                plan_hash_history: vec![plan_hash.to_string()],
                ..EngineRunnerState::default()
            },
            false,
            None,
            None,
            RunnerCheckpointLedger::default(),
            None,
        )),
        Err(error) => Err(RunnerError::CheckpointLoad {
            path: checkpoint_path.display().to_string(),
            reason: error.to_string(),
        }),
    }
}

fn collect_legacy_checkpoint_node_refs(
    checkpoint: &ais_engine::CheckpointDocument,
) -> Option<Vec<String>> {
    let mut refs = Vec::<String>::new();
    refs.extend(checkpoint.engine_state.completed_node_ids.iter().cloned());
    refs.extend(checkpoint.engine_state.pending_retries.keys().cloned());
    refs.extend(
        checkpoint
            .approvals_ledger
            .iter()
            .map(|entry| entry.node_id.clone()),
    );
    refs.extend(
        checkpoint
            .side_effects
            .iter()
            .map(|entry| entry.node_id.clone()),
    );
    if let Some(nodes) = checkpoint
        .plan_snapshot
        .as_ref()
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
    {
        for node in nodes {
            if let Some(node_id) = node.get("id").and_then(Value::as_str) {
                refs.push(node_id.to_string());
            }
        }
    }
    refs.retain(|value| value.contains('/'));
    if refs.is_empty() {
        return None;
    }
    refs.sort();
    refs.dedup();
    Some(refs)
}

fn dedupe_plan_nodes_by_id(plan: &mut PlanDocument) -> usize {
    if plan.nodes.is_empty() {
        return 0;
    }
    let mut seen = HashSet::<String>::new();
    let mut deduped_reversed = Vec::<Value>::with_capacity(plan.nodes.len());
    for node in plan.nodes.iter().rev() {
        let Some(node_id) = node.get("id").and_then(Value::as_str) else {
            deduped_reversed.push(node.clone());
            continue;
        };
        if seen.insert(node_id.to_string()) {
            deduped_reversed.push(node.clone());
        }
    }
    deduped_reversed.reverse();
    let removed = plan.nodes.len().saturating_sub(deduped_reversed.len());
    plan.nodes = deduped_reversed;
    removed
}

fn write_event_sinks(
    command: &AgentCommand,
    events: &[ais_engine::EngineEventRecord],
) -> Result<(), RunnerError> {
    if command.verbose {
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
            if event_type == "need_user_confirm" {
                if let Ok(detail) = serde_json::to_string(&record.event.data) {
                    eprintln!("[policy gate seq={}] output={}", record.seq, detail);
                }
            } else if event_type == "error" {
                let reason_code = record
                    .event
                    .data
                    .get("reason_code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if is_policy_gate_reason_code(reason_code) {
                    if let Ok(detail) = serde_json::to_string(&record.event.data) {
                        eprintln!("[policy gate seq={}] output={}", record.seq, detail);
                    }
                }
            }
        }
    }

    if let Some(target) = &command.events_jsonl {
        if target == "-" {
            for event in events {
                let line = encode_event_jsonl_line(event)
                    .map_err(|error| RunnerError::EventsIo(error.to_string()))?;
                print!("{line}");
            }
        } else {
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

fn is_policy_gate_reason_code(reason_code: &str) -> bool {
    reason_code.starts_with("allowlist_")
        || reason_code.starts_with("threshold_")
        || reason_code.starts_with("missing_")
        || reason_code == "unknown_fields"
        || reason_code.starts_with("hard_block")
}

fn reconcile_pending_side_effects(
    ledger: &mut RunnerCheckpointLedger,
    router: &ais_engine::RouterExecutor,
    state: &mut EngineRunnerState,
) -> Option<String> {
    let pending = ledger.pending_side_effects();
    if pending.is_empty() {
        return None;
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
        return Some(format!(
            "side_effect_reconcile_reverted:{}",
            reverted_nodes.into_iter().collect::<Vec<_>>().join(",")
        ));
    }
    if !pending_nodes.is_empty() {
        return Some(format!(
            "side_effect_reconcile_pending:{}",
            pending_nodes.into_iter().collect::<Vec<_>>().join(",")
        ));
    }
    None
}

pub(super) fn record_side_effect_lifecycle(runtime: &mut Value, ledger: &RunnerCheckpointLedger) {
    record_runtime_agent_field(
        runtime,
        "side_effect_lifecycle",
        ledger.side_effect_lifecycle_summary(),
    );
}

fn maybe_save_checkpoint(
    command: &AgentCommand,
    run_id: &str,
    plan_hash: &str,
    plan: &ais_sdk::PlanDocument,
    state: &EngineRunnerState,
    ledger: &RunnerCheckpointLedger,
    checkpoint_extensions: Option<Map<String, Value>>,
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
    if let Some(extensions) = checkpoint_extensions {
        checkpoint.extensions = extensions;
    }
    save_checkpoint_to_path(path, &checkpoint).map_err(|error| RunnerError::CheckpointSave {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    if command.verbose {
        eprintln!(
            "[checkpoint] saved path={} plan_hash={} epoch={} history={}",
            path.display(),
            plan_hash,
            state.plan_epoch,
            state.plan_hash_history.len()
        );
    }
    Ok(())
}

fn render_agent_output(
    command: &AgentCommand,
    state: &EngineRunnerState,
    status: EngineRunStatus,
    iterations: usize,
    total_events: usize,
    resumed_from_checkpoint: bool,
) -> Result<String, RunnerError> {
    let status_text = match status {
        EngineRunStatus::Completed => "completed",
        EngineRunStatus::Paused => "paused",
        EngineRunStatus::Stopped => "stopped",
    };
    let llm_usage = state.runtime.pointer("/agent/llm_usage").cloned();

    match command.format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&serde_json::json!({
            "schema": "ais-runner-agent/0.0.1",
            "status": status_text,
            "paused_reason": state.paused_reason,
            "resumed_from_checkpoint": resumed_from_checkpoint,
            "iterations": iterations,
            "events_emitted": total_events,
            "llm_usage": llm_usage,
        }))?),
        OutputFormat::Text => {
            let mut output = format!(
                "AIS agent\nstatus: {}\npaused_reason: {}\nresumed_from_checkpoint: {}\niterations: {}\nevents: {}",
                status_text,
                state.paused_reason.clone().unwrap_or_else(|| "none".to_string()),
                resumed_from_checkpoint,
                iterations,
                total_events,
            );
            if let Some(line) = render_llm_usage_line(llm_usage.as_ref()) {
                output.push('\n');
                output.push_str(line.as_str());
            }
            Ok(output)
        }
    }
}

fn render_llm_usage_line(llm_usage: Option<&Value>) -> Option<String> {
    let usage = llm_usage?;
    let calls = usage.get("calls").and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let estimated_calls = usage
        .get("estimated_calls")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source = usage
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut line = format!(
        "llm_usage: calls={} input_tokens={} output_tokens={} total_tokens={} estimated_calls={} source={}",
        calls, input_tokens, output_tokens, total_tokens, estimated_calls, source
    );
    if let Some(limit) = usage.get("context_limit_tokens").and_then(Value::as_u64) {
        let soft_limit = usage
            .get("context_soft_limit_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(limit);
        let context_window_input_tokens = usage
            .get("context_window_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(input_tokens);
        let remaining = usage
            .get("context_remaining_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| soft_limit.saturating_sub(context_window_input_tokens));
        line.push_str(
            format!(
                " context_limit_tokens={} context_soft_limit_tokens={} context_remaining_tokens={}",
                limit, soft_limit, remaining
            )
            .as_str(),
        );
    }
    if let Some(duplicate_ratio_bps) = usage
        .pointer("/diagnostics/duplicate_tool_call_ratio_bps")
        .and_then(Value::as_u64)
    {
        line.push_str(
            format!(
                " duplicate_tool_call_ratio_bps={} discovery_tool_call_ratio_bps={} empty_search_streak_max={}",
                duplicate_ratio_bps,
                usage
                    .pointer("/diagnostics/discovery_tool_call_ratio_bps")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                usage
                    .pointer("/diagnostics/empty_search_streak_max")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
            .as_str(),
        );
    }
    Some(line)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
