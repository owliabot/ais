use super::candidates::CandidateContext;
use super::input_normalize::normalize_input_slot_key;
use super::input_store::{InputStore, VolatileInputSignal};
use ais_sdk::documents::{PlanSketchSegment, PlanSketchStep};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const VOLATILE_FACT_MAX_AGE_MS: u64 = 30_000;

#[derive(Debug, Clone)]
struct WriteGateChainDiagnostics {
    gate_reason_code: &'static str,
    message: String,
    action_depends_on: Vec<String>,
    gate_step_ids: Vec<String>,
    gates_missing_query_dep: Vec<String>,
}

pub(super) fn validate_segment_write_gates(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    input_store: Option<&InputStore>,
) -> Result<(), Value> {
    let mut issues = Vec::<Value>::new();
    let step_by_id = segment
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<BTreeMap<_, _>>();

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
                "reason_code": "missing_query_assert_branch_chain",
                "gate_reason_code": chain_issue.gate_reason_code,
                "message": chain_issue.message,
                "step_id": step.id,
                "candidate_ref": step_candidate_ref,
                "required_pattern": "query -> assert|branch -> action",
                "action_depends_on": chain_issue.action_depends_on,
                "gate_step_ids": chain_issue.gate_step_ids,
                "gates_missing_query_dep": chain_issue.gates_missing_query_dep,
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

        if let Some(store) = input_store {
            if let Some(required_fact) = missing_asset_decimals_fact(step, detail) {
                if !asset_decimals_available(step, segment, candidate_context, store) {
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
                if segment_has_query_for_signal(segment, candidate_context, signal)
                    || store.has_fresh_volatile_signal(
                        signal,
                        VOLATILE_FACT_MAX_AGE_MS,
                        current_unix_ms(),
                    )
                {
                    continue;
                }
                issues.push(json!({
                    "kind": "write_gate_missing",
                    "reason_code": "stale_volatile_fact",
                    "message": format!("volatile fact `{}` is stale or missing; add fresh query in this segment before write", volatile_signal_name(signal)),
                    "step_id": step.id,
                    "candidate_ref": step_candidate_ref,
                    "required_signal": volatile_signal_name(signal),
                    "max_age_ms": VOLATILE_FACT_MAX_AGE_MS,
                }));
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
            gate_reason_code: "missing_action_gate_dep",
            message: "write action is missing assert/branch gate dependency in depends_on"
                .to_string(),
            action_depends_on,
            gate_step_ids: Vec::new(),
            gates_missing_query_dep: Vec::new(),
        });
    }
    let gate_step_ids = reachable_gate_step_ids(action_step, step_by_id);
    if gate_step_ids.is_empty() {
        return Some(WriteGateChainDiagnostics {
            gate_reason_code: "missing_action_gate_dep",
            message:
                "write action depends_on does not include any assert/branch gate step in dependency ancestry"
                    .to_string(),
            action_depends_on,
            gate_step_ids,
            gates_missing_query_dep: Vec::new(),
        });
    }
    let gates_missing_query_dep = gate_step_ids
        .iter()
        .filter(|gate_id| !gate_has_query_backing(gate_id.as_str(), step_by_id, candidate_context))
        .cloned()
        .collect::<Vec<_>>();
    if gates_missing_query_dep.len() == gate_step_ids.len() {
        return Some(WriteGateChainDiagnostics {
            gate_reason_code: "missing_gate_query_dep",
            message:
                "assert/branch gate steps must depend_on query facts in the same segment before write actions"
                    .to_string(),
            action_depends_on,
            gate_step_ids,
            gates_missing_query_dep,
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

fn gate_has_query_backing<'a>(
    step_id: &'a str,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
) -> bool {
    let mut visited = HashSet::<String>::new();
    gate_has_query_backing_inner(step_id, step_by_id, candidate_context, &mut visited)
}

fn gate_has_query_backing_inner<'a>(
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

    step.depends_on.iter().any(|dep| {
        gate_has_query_backing_inner(dep.as_str(), step_by_id, candidate_context, visited)
    })
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
    let queries = detail.get("requires_queries")?.as_array()?;
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
    let Some(detail) = detail else {
        return None;
    };
    let Some(params) = detail.get("params").and_then(Value::as_array) else {
        return None;
    };

    params
        .iter()
        .filter_map(asset_param_slot)
        .find(|slot| {
            action_step
                .inputs
                .get(slot.as_str())
                .is_some_and(|value| !value_contains_asset_decimals(value))
        })
        .map(|slot| format!("{slot}.decimals"))
}

fn asset_decimals_available(
    action_step: &PlanSketchStep,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    input_store: &InputStore,
) -> bool {
    if action_step
        .inputs
        .values()
        .any(value_contains_asset_decimals)
    {
        return true;
    }
    if input_store
        .list_ref_strings()
        .iter()
        .any(|slot| slot.ends_with(".decimals"))
    {
        return true;
    }
    if query_steps_store_asset_decimals(segment, candidate_context) {
        return true;
    }
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
        query_candidate_returns_decimals(reference, candidate_context)
    })
}

fn query_steps_store_asset_decimals(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
) -> bool {
    segment.steps.iter().any(|step| {
        if step.kind != "query" {
            return false;
        }
        let slot_has_asset_decimals = step
            .stores
            .values()
            .any(|slot| slot_is_asset_decimals(slot.as_str()));
        if slot_has_asset_decimals {
            return true;
        }
        let Some(reference) = step_candidate_ref(step) else {
            return false;
        };
        is_query_candidate_ref(reference, candidate_context)
            && query_candidate_returns_decimals(reference, candidate_context)
    })
}

fn slot_is_asset_decimals(slot: &str) -> bool {
    let Some(canonical) = normalize_input_slot_key(slot) else {
        return false;
    };
    let lowered = canonical.to_lowercase();
    lowered.ends_with(".decimals")
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

fn query_candidate_returns_decimals(
    candidate_ref: &str,
    candidate_context: &CandidateContext,
) -> bool {
    if candidate_leaf_name(candidate_ref).contains("decimal") {
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
                    .map(|name| name.to_lowercase().contains("decimal"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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

fn value_contains_asset_decimals(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(decimals) = object.get("decimals") {
                if decimals.is_number() || decimals.is_string() {
                    return true;
                }
            }
            if let Some(lit) = object.get("lit") {
                if value_contains_asset_decimals(lit) {
                    return true;
                }
            }
            if let Some(inner_object) = object.get("object") {
                return value_contains_asset_decimals(inner_object);
            }
            false
        }
        Value::Array(values) => values.iter().any(value_contains_asset_decimals),
        _ => false,
    }
}

fn volatile_signal_name(signal: VolatileInputSignal) -> &'static str {
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
