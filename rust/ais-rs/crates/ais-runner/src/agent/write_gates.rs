use super::candidates::CandidateContext;
use super::input_normalize::normalize_input_slot_key;
use super::input_store::{InputStore, VolatileInputSignal, VolatileSignalObservation};
use super::runtime_facts_store::RuntimeFactsStore;
use crate::policy::VolatileFactsPolicy;
use ais_sdk::documents::{PlanSketchSegment, PlanSketchStep};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const WRITE_GATE_CHAIN_FAMILY_REASON_CODE: &str = "missing_query_assert_branch_chain";
const WRITE_GATE_ACCEPTED_BACKING_MODES: [&str; 2] =
    ["same_segment_query", "historical_node_output"];

#[derive(Debug, Clone)]
struct WriteGateChainDiagnostics {
    reason_code: &'static str,
    family_reason_code: &'static str,
    message: String,
    action_depends_on: Vec<String>,
    gate_step_ids: Vec<String>,
    missing_depends_on: bool,
    missing_gate_step_ids: Vec<String>,
    missing_data_backing_refs: Vec<String>,
}

#[cfg(test)]
pub(super) fn validate_segment_write_gates(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Result<(), Value> {
    validate_segment_write_gates_with_policy(
        segment,
        candidate_context,
        runtime_facts_store,
        input_store,
        VolatileFactsPolicy::default(),
    )
}

pub(super) fn validate_segment_write_gates_with_policy(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
    volatile_facts_policy: VolatileFactsPolicy,
) -> Result<(), Value> {
    let mut issues = Vec::<Value>::new();
    let step_by_id = segment
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<BTreeMap<_, _>>();
    let now_ms = current_unix_ms();

    for step in &segment.steps {
        if step.kind != "action" {
            continue;
        }
        let Some(step_candidate_ref) = step_candidate_ref(step) else {
            issues.push(json!({
                "kind": "write_gate_missing",
                "reason_code": "missing_action_candidate_ref",
                "message": "action step missing candidate_ref",
                "step_id": step.id,
            }));
            continue;
        };
        let detail = candidate_context.detail_by_ref.get(step_candidate_ref);
        if !action_requires_write_gate(detail) {
            continue;
        }

        if let Some(chain_issue) =
            diagnose_required_write_gate_chain(step, &step_by_id, candidate_context)
        {
            issues.push(json!({
                "kind": "write_gate_missing",
                "reason_code": chain_issue.reason_code,
                "family_reason_code": chain_issue.family_reason_code,
                "message": chain_issue.message,
                "step_id": step.id,
                "candidate_ref": step_candidate_ref,
                "required_pattern": "assert|branch -> action with query ancestry or explicit historical node-output backing",
                "action_depends_on": chain_issue.action_depends_on,
                "gate_step_ids": chain_issue.gate_step_ids,
                "missing_depends_on": chain_issue.missing_depends_on,
                "missing_gate_step_ids": chain_issue.missing_gate_step_ids,
                "missing_data_backing_refs": chain_issue.missing_data_backing_refs,
                "accepted_backing_modes": WRITE_GATE_ACCEPTED_BACKING_MODES,
            }));
        }

        if let Some(required_queries) = action_required_queries(detail) {
            for query_name in required_queries {
                if !segment_has_query_name(segment, candidate_context, query_name.as_str()) {
                    issues.push(json!({
                        "kind": "write_gate_missing",
                        "reason_code": "missing_required_query",
                        "message": format!("action requires query `{query_name}` before execution"),
                        "step_id": step.id,
                        "candidate_ref": step_candidate_ref,
                        "required_query": query_name,
                    }));
                }
            }
        }

        if let Some(required_fact) = missing_asset_decimals_fact(step, detail) {
            if !asset_decimals_available(step, detail, runtime_facts_store, input_store) {
                issues.push(json!({
                    "kind": "write_gate_missing",
                    "reason_code": "missing_token_decimals",
                    "message": "asset decimals unavailable; add decimals query or return missing_required_input",
                    "step_id": step.id,
                    "candidate_ref": step_candidate_ref,
                    "required_fact": required_fact,
                    "required_object_fields": ["decimals"],
                }));
            }
        }

        for signal in required_volatile_signals(step, detail) {
            if let Some(issue) = stale_volatile_signal_issue(
                step,
                step_candidate_ref,
                segment,
                candidate_context,
                runtime_facts_store,
                input_store,
                signal,
                now_ms,
                volatile_facts_policy,
            ) {
                issues.push(issue);
            }
        }
    }

    if issues.is_empty() {
        return Ok(());
    }
    Err(json!({
        "reason_code": "write_gate_missing",
        "message": "segment write preconditions are not satisfied",
        "issues": issues,
    }))
}

fn stale_volatile_signal_issue(
    step: &PlanSketchStep,
    step_candidate_ref: &str,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
    signal: VolatileInputSignal,
    now_ms: u64,
    volatile_facts_policy: VolatileFactsPolicy,
) -> Option<Value> {
    if segment_has_query_for_signal(segment, candidate_context, signal) {
        return None;
    }
    if action_has_historical_node_output_backing_for_signal(step, segment, signal) {
        return None;
    }
    let max_age_ms = volatile_facts_policy.max_age_ms;
    let has_fresh_fact = runtime_facts_store
        .is_some_and(|store| store.has_fresh_volatile_signal(signal, max_age_ms, now_ms))
        || input_store
            .is_some_and(|store| store.has_fresh_volatile_signal(signal, max_age_ms, now_ms));
    if has_fresh_fact {
        return None;
    }
    let latest_observation =
        freshest_volatile_signal_observation(runtime_facts_store, input_store, signal);
    let observed_at_ms = latest_observation.map(|observation| observation.observed_at_ms);
    let age_ms = observed_at_ms.map(|observed_at_ms| now_ms.saturating_sub(observed_at_ms));
    Some(json!({
        "kind": "write_gate_missing",
        "reason_code": "stale_volatile_fact",
        "message": format!("volatile fact `{}` is stale or missing; add fresh query in this segment before write", volatile_signal_name(signal)),
        "step_id": step.id,
        "candidate_ref": step_candidate_ref,
        "required_signal": volatile_signal_name(signal),
        "observed_at_ms": observed_at_ms,
        "age_ms": age_ms,
        "max_age_ms": max_age_ms,
    }))
}

fn action_has_historical_node_output_backing_for_signal(
    action_step: &PlanSketchStep,
    segment: &PlanSketchSegment,
    signal: VolatileInputSignal,
) -> bool {
    let step_by_id = segment
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<BTreeMap<_, _>>();
    reachable_gate_step_ids(action_step, &step_by_id)
        .into_iter()
        .filter_map(|step_id| step_by_id.get(step_id.as_str()).copied())
        .any(|gate_step| step_references_node_output_for_signal(gate_step, signal))
}

fn freshest_volatile_signal_observation(
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
    signal: VolatileInputSignal,
) -> Option<VolatileSignalObservation> {
    runtime_facts_store
        .and_then(|store| store.newest_volatile_signal_observation(signal))
        .into_iter()
        .chain(input_store.and_then(|store| store.newest_volatile_signal_observation(signal)))
        .max_by_key(|observation| observation.observed_at_ms)
}

fn action_requires_write_gate(detail: Option<&Value>) -> bool {
    let Some(detail) = detail else {
        return false;
    };
    if detail.get("kind").and_then(Value::as_str) != Some("action") {
        return false;
    }
    if let Some(required) = explicit_write_gate_required(detail) {
        return required;
    }
    action_required_queries(Some(detail)).is_some()
        || action_has_asset_param(detail)
        || action_has_write_risk_tags(detail)
        || action_has_write_param_roles(detail)
}

fn explicit_write_gate_required(detail: &Value) -> Option<bool> {
    if let Some(required) = detail
        .pointer("/write_gate/required")
        .and_then(Value::as_bool)
    {
        return Some(required);
    }
    if let Some(required) = detail
        .pointer("/extensions/write_gate/required")
        .and_then(Value::as_bool)
    {
        return Some(required);
    }
    let mode = detail
        .pointer("/write_gate/mode")
        .and_then(Value::as_str)
        .or_else(|| {
            detail
                .pointer("/extensions/write_gate/mode")
                .and_then(Value::as_str)
        })?;
    match mode.trim().to_lowercase().as_str() {
        "required" => Some(true),
        "none" | "disabled" | "off" => Some(false),
        _ => None,
    }
}

fn action_has_asset_param(detail: &Value) -> bool {
    detail
        .get("params")
        .and_then(Value::as_array)
        .map(|params| {
            params.iter().any(|param| {
                param
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|param_type| param_type.eq_ignore_ascii_case("asset"))
            })
        })
        .unwrap_or(false)
}

fn action_has_write_param_roles(detail: &Value) -> bool {
    detail
        .get("params")
        .and_then(Value::as_array)
        .map(|params| {
            params.iter().any(|param| {
                let Some(role) = param
                    .get("role")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_lowercase())
                else {
                    return false;
                };
                matches!(
                    role.as_str(),
                    "spend_amount" | "approval_amount" | "spender_address"
                )
            })
        })
        .unwrap_or(false)
}

fn action_has_write_risk_tags(detail: &Value) -> bool {
    detail
        .get("risk_tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter().any(|tag| {
                let Some(tag) = tag
                    .as_str()
                    .map(|value| value.trim().to_lowercase())
                    .filter(|value| !value.is_empty())
                else {
                    return false;
                };
                matches!(
                    tag.as_str(),
                    "transfer"
                        | "token_transfer"
                        | "native_transfer"
                        | "swap"
                        | "token_swap"
                        | "approve"
                        | "approval"
                        | "write"
                )
            })
        })
        .unwrap_or(false)
}

fn action_has_allowance_like_risk_tags(detail: &Value) -> bool {
    detail
        .get("risk_tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter().any(|tag| {
                let Some(tag) = tag
                    .as_str()
                    .map(|value| value.trim().to_lowercase())
                    .filter(|value| !value.is_empty())
                else {
                    return false;
                };
                matches!(tag.as_str(), "approve" | "approval" | "allowance")
            })
        })
        .unwrap_or(false)
}

fn diagnose_required_write_gate_chain<'a>(
    action_step: &'a PlanSketchStep,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
) -> Option<WriteGateChainDiagnostics> {
    let action_depends_on = action_step.depends_on.clone();
    if action_depends_on.is_empty() {
        return Some(WriteGateChainDiagnostics {
            reason_code: "missing_action_gate_dep",
            family_reason_code: WRITE_GATE_CHAIN_FAMILY_REASON_CODE,
            message:
                "write action is missing an assert/branch gate in depends_on; add a gate step before the action"
                    .to_string(),
            action_depends_on,
            gate_step_ids: Vec::new(),
            missing_depends_on: true,
            missing_gate_step_ids: Vec::new(),
            missing_data_backing_refs: Vec::new(),
        });
    }
    let gate_step_ids = reachable_gate_step_ids(action_step, step_by_id);
    if gate_step_ids.is_empty() {
        return Some(WriteGateChainDiagnostics {
            reason_code: "missing_action_gate_dep",
            family_reason_code: WRITE_GATE_CHAIN_FAMILY_REASON_CODE,
            message:
                "write action depends_on does not reach any assert/branch gate step; add a gate step to the dependency chain"
                    .to_string(),
            action_depends_on,
            gate_step_ids,
            missing_depends_on: true,
            missing_gate_step_ids: Vec::new(),
            missing_data_backing_refs: Vec::new(),
        });
    }
    let missing_gate_step_ids = gate_step_ids
        .iter()
        .filter(|gate_id| !gate_has_data_backing(gate_id.as_str(), step_by_id, candidate_context))
        .cloned()
        .collect::<Vec<_>>();
    if missing_gate_step_ids.len() == gate_step_ids.len() {
        let missing_gate_label = missing_gate_step_ids.join(", ");
        return Some(WriteGateChainDiagnostics {
            reason_code: "missing_gate_data_backing",
            family_reason_code: WRITE_GATE_CHAIN_FAMILY_REASON_CODE,
            message: format!(
                "gate step(s) [{missing_gate_label}] are not backed by same-segment query ancestry or historical node outputs; add one accepted backing before the write"
            ),
            action_depends_on,
            gate_step_ids,
            missing_depends_on: false,
            missing_gate_step_ids,
            missing_data_backing_refs: Vec::new(),
        });
    }
    None
}

fn reachable_gate_step_ids<'a>(
    action_step: &'a PlanSketchStep,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
) -> Vec<String> {
    let mut gate_ids = BTreeSet::<String>::new();
    let mut visited = HashSet::<String>::new();
    for dep in &action_step.depends_on {
        collect_reachable_gate_step_ids(dep.as_str(), step_by_id, &mut visited, &mut gate_ids);
    }
    gate_ids.into_iter().collect::<Vec<_>>()
}

fn collect_reachable_gate_step_ids<'a>(
    step_id: &'a str,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    visited: &mut HashSet<String>,
    gate_ids: &mut BTreeSet<String>,
) {
    if !visited.insert(step_id.to_string()) {
        return;
    }
    let Some(step) = step_by_id.get(step_id).copied() else {
        return;
    };
    if step.kind == "assert" || step.kind == "branch" {
        gate_ids.insert(step.id.clone());
    }
    for dep in &step.depends_on {
        collect_reachable_gate_step_ids(dep.as_str(), step_by_id, visited, gate_ids);
    }
}

fn gate_has_data_backing<'a>(
    step_id: &'a str,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
) -> bool {
    let mut visited = HashSet::<String>::new();
    gate_has_data_backing_inner(step_id, step_by_id, candidate_context, &mut visited)
}

fn gate_has_data_backing_inner<'a>(
    step_id: &'a str,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
    visited: &mut HashSet<String>,
) -> bool {
    if !visited.insert(step_id.to_string()) {
        return false;
    }
    let Some(step) = step_by_id.get(step_id).copied() else {
        return false;
    };

    if step.kind == "query" {
        return step_candidate_ref(step)
            .is_some_and(|reference| is_query_candidate_ref(reference, candidate_context));
    }

    if step.kind == "assert" || step.kind == "branch" {
        if step_explicitly_references_node_outputs(step) {
            return true;
        }
    }

    step.depends_on.iter().any(|dep| {
        gate_has_data_backing_inner(dep.as_str(), step_by_id, candidate_context, visited)
    })
}

fn step_explicitly_references_node_outputs(step: &PlanSketchStep) -> bool {
    if let Some(when) = step.when.as_ref() {
        if text_explicitly_references_node_outputs(when.cel.as_str()) {
            return true;
        }
    }

    serde_json::to_value(&step.inputs)
        .ok()
        .is_some_and(|value| value_explicitly_references_node_outputs(&value))
}

fn step_references_node_output_for_signal(
    step: &PlanSketchStep,
    signal: VolatileInputSignal,
) -> bool {
    if let Some(when) = step.when.as_ref() {
        if text_references_node_output_for_signal(when.cel.as_str(), signal) {
            return true;
        }
    }
    serde_json::to_value(&step.inputs)
        .ok()
        .is_some_and(|value| value_references_node_output_for_signal(&value, signal))
}

fn value_explicitly_references_node_outputs(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, inner)| {
            if key == "ref" {
                return inner.as_str().is_some_and(|reference| {
                    reference.starts_with("nodes.") && reference.contains(".outputs.")
                });
            }
            if key == "cel" {
                return inner
                    .as_str()
                    .is_some_and(text_explicitly_references_node_outputs);
            }
            value_explicitly_references_node_outputs(inner)
        }),
        Value::Array(items) => items.iter().any(value_explicitly_references_node_outputs),
        _ => false,
    }
}

fn value_references_node_output_for_signal(value: &Value, signal: VolatileInputSignal) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, inner)| {
            if key == "ref" {
                return inner
                    .as_str()
                    .is_some_and(|reference| node_output_ref_matches_signal(reference, signal));
            }
            if key == "cel" {
                return inner
                    .as_str()
                    .is_some_and(|text| text_references_node_output_for_signal(text, signal));
            }
            value_references_node_output_for_signal(inner, signal)
        }),
        Value::Array(items) => items
            .iter()
            .any(|item| value_references_node_output_for_signal(item, signal)),
        _ => false,
    }
}

fn text_explicitly_references_node_outputs(text: &str) -> bool {
    text.find("nodes.")
        .zip(text.find(".outputs."))
        .is_some_and(|(nodes_pos, outputs_pos)| outputs_pos > nodes_pos + "nodes.".len())
}

fn text_references_node_output_for_signal(text: &str, signal: VolatileInputSignal) -> bool {
    text.match_indices("nodes.").any(|(start, _)| {
        node_output_field_name(&text[start..])
            .is_some_and(|field| leaf_matches_signal(field, signal))
    })
}

fn node_output_ref_matches_signal(reference: &str, signal: VolatileInputSignal) -> bool {
    reference.starts_with("nodes.")
        && node_output_field_name(reference).is_some_and(|field| leaf_matches_signal(field, signal))
}

fn node_output_field_name(reference: &str) -> Option<&str> {
    let outputs_marker = ".outputs.";
    let outputs_start = reference.find(outputs_marker)? + outputs_marker.len();
    let tail = reference.get(outputs_start..)?;
    let field_end = tail
        .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .unwrap_or(tail.len());
    let field = tail.get(..field_end)?.trim();
    if field.is_empty() {
        return None;
    }
    Some(field)
}

fn is_query_candidate_ref(candidate_ref: &str, candidate_context: &CandidateContext) -> bool {
    if let Some(detail) = candidate_context.detail_by_ref.get(candidate_ref) {
        if detail.get("kind").and_then(Value::as_str) == Some("query") {
            return true;
        }
    }
    candidate_context
        .executable_candidates
        .queries
        .iter()
        .any(|card| card.get("ref").and_then(Value::as_str) == Some(candidate_ref))
}

fn action_required_queries(detail: Option<&Value>) -> Option<Vec<String>> {
    let detail = detail?;
    let queries = detail
        .pointer("/semantic_hints/prerequisites/requires_queries")
        .and_then(Value::as_array)
        .or_else(|| detail.get("requires_queries").and_then(Value::as_array))?;
    let names = queries
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
}

fn segment_has_query_name(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    required_query_name: &str,
) -> bool {
    let target = required_query_name.to_lowercase();
    segment.steps.iter().any(|step| {
        if step.kind != "query" && step.kind != "assert" {
            return false;
        }
        let Some(reference) = step_candidate_ref(step) else {
            return false;
        };
        if !is_query_candidate_ref(reference, candidate_context) {
            return false;
        }
        candidate_leaf_name(reference) == target
    })
}

fn segment_has_query_for_signal(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    signal: VolatileInputSignal,
) -> bool {
    segment.steps.iter().any(|step| {
        if step.kind != "query" {
            return false;
        }
        let Some(reference) = step_candidate_ref(step) else {
            return false;
        };
        if !is_query_candidate_ref(reference, candidate_context) {
            return false;
        }
        query_candidate_matches_signal(reference, candidate_context, signal)
    })
}

fn missing_asset_decimals_fact(
    action_step: &PlanSketchStep,
    detail: Option<&Value>,
) -> Option<String> {
    action_asset_slots_missing_decimals(action_step, detail)
        .into_iter()
        .next()
        .map(|slot| format!("{slot}.decimals"))
}

fn asset_decimals_available(
    action_step: &PlanSketchStep,
    detail: Option<&Value>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> bool {
    let decimals_max = super::missing_resolution::heuristics::token_decimals_max();
    let missing_slots = action_asset_slots_missing_decimals(action_step, detail);
    if missing_slots.is_empty() {
        return true;
    }
    missing_slots.iter().all(|slot| {
        action_step.inputs.get(slot.as_str()).is_some_and(|value| {
            asset_value_contains_resolved_valid_decimals(
                value,
                runtime_facts_store,
                input_store,
                decimals_max,
            )
        }) || {
            let canonical_key = format!("inputs.{slot}.decimals");
            runtime_facts_has_valid_decimals(
                runtime_facts_store,
                canonical_key.as_str(),
                decimals_max,
            ) || input_store_has_valid_decimals(input_store, canonical_key.as_str(), decimals_max)
        }
    })
}

fn asset_value_contains_resolved_valid_decimals(
    value: &Value,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
    max: u32,
) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(decimals) = object.get("decimals") {
                if asset_value_contains_resolved_valid_decimals(
                    decimals,
                    runtime_facts_store,
                    input_store,
                    max,
                ) {
                    return true;
                }
            }
            if let Some(lit) = object.get("lit") {
                if asset_value_contains_resolved_valid_decimals(
                    lit,
                    runtime_facts_store,
                    input_store,
                    max,
                ) {
                    return true;
                }
            }
            if let Some(inner_object) = object.get("object") {
                if asset_value_contains_resolved_valid_decimals(
                    inner_object,
                    runtime_facts_store,
                    input_store,
                    max,
                ) {
                    return true;
                }
            }
            if let Some(inner_value) = object.get("value") {
                if asset_value_contains_resolved_valid_decimals(
                    inner_value,
                    runtime_facts_store,
                    input_store,
                    max,
                ) {
                    return true;
                }
            }
            if let Some(ref_path) = object.get("ref").and_then(Value::as_str) {
                if let Some(resolved) =
                    resolve_runtime_ref_value(ref_path, runtime_facts_store, input_store)
                {
                    return asset_value_contains_resolved_valid_decimals(
                        &resolved,
                        runtime_facts_store,
                        input_store,
                        max,
                    );
                }
            }
            super::missing_resolution::heuristics::parse_valid_token_decimals(value, max).is_some()
        }
        Value::Array(values) => values.iter().any(|item| {
            asset_value_contains_resolved_valid_decimals(
                item,
                runtime_facts_store,
                input_store,
                max,
            )
        }),
        _ => {
            super::missing_resolution::heuristics::parse_valid_token_decimals(value, max).is_some()
        }
    }
}

fn resolve_runtime_ref_value(
    ref_path: &str,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    input_store: Option<&InputStore>,
) -> Option<Value> {
    let trimmed = ref_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("inputs.") {
        return input_store
            .and_then(|store| store.get_projected(trimmed))
            .map(|entry| entry.value);
    }
    if trimmed.starts_with("facts.") {
        return runtime_facts_store
            .and_then(|store| store.get(trimmed))
            .map(|entry| entry.value.clone());
    }
    None
}

fn action_asset_slots_missing_decimals(
    action_step: &PlanSketchStep,
    detail: Option<&Value>,
) -> Vec<String> {
    let Some(detail) = detail else {
        return Vec::new();
    };
    let Some(params) = detail.get("params").and_then(Value::as_array) else {
        return Vec::new();
    };
    let decimals_max = super::missing_resolution::heuristics::token_decimals_max();
    params
        .iter()
        .filter_map(asset_param_slot)
        .filter(|slot| {
            action_step.inputs.get(slot.as_str()).is_some_and(|value| {
                !super::missing_resolution::heuristics::value_contains_valid_asset_decimals(
                    value,
                    decimals_max,
                )
            })
        })
        .collect::<Vec<_>>()
}

fn asset_param_slot(param: &Value) -> Option<String> {
    let param_type = param.get("type").and_then(Value::as_str)?;
    if !param_type.eq_ignore_ascii_case("asset") {
        return None;
    }

    let explicit_slot = [
        "input_slot",
        "input_ref",
        "slot",
        "source_slot",
        "source_ref",
        "ref",
    ]
    .into_iter()
    .find_map(|key| param.get(key).and_then(Value::as_str))
    .and_then(normalize_input_slot_key);
    if explicit_slot.is_some() {
        return explicit_slot;
    }

    param
        .get("name")
        .and_then(Value::as_str)
        .and_then(normalize_input_slot_key)
}

fn step_candidate_ref(step: &PlanSketchStep) -> Option<&str> {
    step.candidate_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn query_candidate_matches_signal(
    candidate_ref: &str,
    candidate_context: &CandidateContext,
    signal: VolatileInputSignal,
) -> bool {
    let leaf_name = candidate_leaf_name(candidate_ref);
    if leaf_matches_signal(leaf_name.as_str(), signal) {
        return true;
    }
    candidate_context
        .detail_by_ref
        .get(candidate_ref)
        .and_then(|detail| detail.get("returns"))
        .and_then(Value::as_array)
        .map(|returns| {
            returns.iter().any(|entry| {
                entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| leaf_matches_signal(name, signal))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn leaf_matches_signal(name: &str, signal: VolatileInputSignal) -> bool {
    let lowered = name.to_lowercase();
    match signal {
        VolatileInputSignal::Balance => lowered.contains("balance"),
        VolatileInputSignal::Allowance => lowered.contains("allowance"),
    }
}

fn required_volatile_signals(
    step: &PlanSketchStep,
    detail: Option<&Value>,
) -> Vec<VolatileInputSignal> {
    let Some(detail) = detail else {
        return Vec::new();
    };
    let mut out = BTreeSet::<String>::new();

    if action_has_write_risk_tags(detail) {
        out.insert("balance".to_string());
    }
    if action_has_allowance_like_risk_tags(detail) || action_has_allowance_role(detail) {
        out.insert("allowance".to_string());
    }
    if let Some(required_queries) = action_required_queries(Some(detail)) {
        for required_query in required_queries {
            let lowered = required_query.to_lowercase();
            if lowered.contains("balance") {
                out.insert("balance".to_string());
            }
            if lowered.contains("allowance") {
                out.insert("allowance".to_string());
            }
        }
    }
    if missing_asset_decimals_fact(step, Some(detail)).is_some() {
        out.insert("balance".to_string());
    }

    out.into_iter()
        .filter_map(|signal| match signal.as_str() {
            "balance" => Some(VolatileInputSignal::Balance),
            "allowance" => Some(VolatileInputSignal::Allowance),
            _ => None,
        })
        .collect::<Vec<_>>()
}

pub(super) fn required_action_volatile_signals(
    step: &PlanSketchStep,
    candidate_context: &CandidateContext,
) -> Vec<VolatileInputSignal> {
    if step.kind != "action" {
        return Vec::new();
    }
    let detail = step_candidate_ref(step)
        .and_then(|candidate_ref| candidate_context.detail_by_ref.get(candidate_ref));
    required_volatile_signals(step, detail)
}

fn action_has_allowance_role(detail: &Value) -> bool {
    detail
        .get("params")
        .and_then(Value::as_array)
        .map(|params| {
            params.iter().any(|param| {
                let Some(role) = param
                    .get("role")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_lowercase())
                else {
                    return false;
                };
                matches!(role.as_str(), "approval_amount" | "spender_address")
            })
        })
        .unwrap_or(false)
}

fn candidate_leaf_name(candidate_ref: &str) -> String {
    candidate_ref
        .split('/')
        .nth(1)
        .unwrap_or(candidate_ref)
        .trim()
        .to_lowercase()
}

fn runtime_facts_has_valid_decimals(
    runtime_facts_store: Option<&RuntimeFactsStore>,
    key: &str,
    max: u32,
) -> bool {
    runtime_facts_store
        .and_then(|store| store.get(key))
        .and_then(|entry| {
            super::missing_resolution::heuristics::parse_valid_token_decimals(&entry.value, max)
        })
        .is_some()
}

fn input_store_has_valid_decimals(input_store: Option<&InputStore>, key: &str, max: u32) -> bool {
    input_store
        .and_then(|store| store.get_projected(key))
        .and_then(|entry| {
            super::missing_resolution::heuristics::parse_valid_token_decimals(&entry.value, max)
        })
        .is_some()
}

pub(super) fn volatile_signal_name(signal: VolatileInputSignal) -> &'static str {
    match signal {
        VolatileInputSignal::Balance => "balance",
        VolatileInputSignal::Allowance => "allowance",
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/write_gates_module.rs"]
mod tests;
