use super::candidates::CandidateContext;
use super::facts::{FactStore, VolatileFactSignal};
use ais_sdk::documents::{PlanSketchSegment, PlanSketchStep};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

const VOLATILE_FACT_MAX_AGE_MS: u64 = 30_000;

pub(super) fn validate_segment_write_gates(
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    fact_store: Option<&FactStore>,
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

        if !has_required_write_gate_chain(step, &step_by_id, candidate_context) {
            issues.push(json!({
                "kind": "write_gate_missing",
                "reason_code": "missing_query_assert_branch_chain",
                "message": "write action must depend on assert/branch gate backed by query facts in the same segment",
                "step_id": step.id,
                "candidate_ref": step_candidate_ref,
                "required_pattern": "query -> assert|branch -> action",
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

        if let Some(store) = fact_store {
            if action_has_asset_inputs_without_decimals(step, detail)
                && !token_decimals_available(step, segment, candidate_context, store)
            {
                issues.push(json!({
                    "kind": "write_gate_missing",
                    "reason_code": "missing_token_decimals",
                    "message": "token decimals unavailable; add decimals query (e.g. erc20/decimals) or return missing_required_input",
                    "step_id": step.id,
                    "candidate_ref": step_candidate_ref,
                    "required_fact": "token.decimals",
                }));
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

fn has_required_write_gate_chain<'a>(
    action_step: &'a PlanSketchStep,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
) -> bool {
    if action_step.depends_on.is_empty() {
        return false;
    }

    let mut visited = HashSet::<String>::new();
    for dep in &action_step.depends_on {
        if has_query_backed_gate_path(
            dep.as_str(),
            false,
            step_by_id,
            candidate_context,
            &mut visited,
        ) {
            return true;
        }
    }
    false
}

fn has_query_backed_gate_path<'a>(
    step_id: &'a str,
    seen_gate: bool,
    step_by_id: &BTreeMap<&'a str, &'a PlanSketchStep>,
    candidate_context: &CandidateContext,
    visited: &mut HashSet<String>,
) -> bool {
    if !visited.insert(format!("{step_id}|{seen_gate}")) {
        return false;
    }
    let Some(step) = step_by_id.get(step_id).copied() else {
        return false;
    };

    if step.kind == "query" {
        return seen_gate
            && step_candidate_ref(step)
                .is_some_and(|reference| is_query_candidate_ref(reference, candidate_context));
    }

    let is_gate = step.kind == "assert" || step.kind == "branch";
    let next_seen_gate = seen_gate || is_gate;
    if next_seen_gate
        && step
            .candidate_ref
            .as_deref()
            .is_some_and(|reference| is_query_candidate_ref(reference, candidate_context))
    {
        return true;
    }

    step.depends_on.iter().any(|dep| {
        has_query_backed_gate_path(
            dep.as_str(),
            next_seen_gate,
            step_by_id,
            candidate_context,
            visited,
        )
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
    signal: VolatileFactSignal,
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

fn action_has_asset_inputs_without_decimals(
    action_step: &PlanSketchStep,
    detail: Option<&Value>,
) -> bool {
    let Some(detail) = detail else {
        return false;
    };
    let Some(params) = detail.get("params").and_then(Value::as_array) else {
        return false;
    };

    params
        .iter()
        .filter_map(|param| {
            let param_type = param.get("type").and_then(Value::as_str)?;
            if !param_type.eq_ignore_ascii_case("asset") {
                return None;
            }
            param.get("name").and_then(Value::as_str)
        })
        .any(|name| {
            action_step
                .inputs
                .get(name)
                .is_some_and(|value| !value_contains_token_decimals(value))
        })
}

fn token_decimals_available(
    action_step: &PlanSketchStep,
    segment: &PlanSketchSegment,
    candidate_context: &CandidateContext,
    fact_store: &FactStore,
) -> bool {
    if action_step
        .inputs
        .values()
        .any(value_contains_token_decimals)
    {
        return true;
    }
    if fact_store.any_key_ends_with(".decimals") {
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
    signal: VolatileFactSignal,
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

fn leaf_matches_signal(name: &str, signal: VolatileFactSignal) -> bool {
    let lowered = name.to_lowercase();
    match signal {
        VolatileFactSignal::Balance => lowered.contains("balance"),
        VolatileFactSignal::Allowance => lowered.contains("allowance"),
    }
}

fn required_volatile_signals(
    step: &PlanSketchStep,
    detail: Option<&Value>,
) -> Vec<VolatileFactSignal> {
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
    if action_has_asset_inputs_without_decimals(step, Some(detail)) {
        out.insert("balance".to_string());
    }

    out.into_iter()
        .filter_map(|signal| match signal.as_str() {
            "balance" => Some(VolatileFactSignal::Balance),
            "allowance" => Some(VolatileFactSignal::Allowance),
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

fn value_contains_token_decimals(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(decimals) = object.get("decimals") {
                if decimals.is_number() || decimals.is_string() {
                    return true;
                }
            }
            if let Some(lit) = object.get("lit") {
                if value_contains_token_decimals(lit) {
                    return true;
                }
            }
            if let Some(inner_object) = object.get("object") {
                return value_contains_token_decimals(inner_object);
            }
            false
        }
        Value::Array(values) => values.iter().any(value_contains_token_decimals),
        _ => false,
    }
}

fn volatile_signal_name(signal: VolatileFactSignal) -> &'static str {
    match signal {
        VolatileFactSignal::Balance => "balance",
        VolatileFactSignal::Allowance => "allowance",
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn candidate_context_with_demo_refs() -> CandidateContext {
        let mut context = CandidateContext::default();
        context.detail_by_ref.insert(
            "demo-bank@0.0.1/native-balance".to_string(),
            json!({
                "ref": "demo-bank@0.0.1/native-balance",
                "kind": "query"
            }),
        );
        context.detail_by_ref.insert(
            "demo-bank@0.0.1/native-transfer".to_string(),
            json!({
                "ref": "demo-bank@0.0.1/native-transfer",
                "kind": "action",
                "risk_tags": ["transfer"]
            }),
        );
        context
    }

    #[test]
    fn write_gate_accepts_recursive_query_assert_branch_action_chain() {
        let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"q_balance","kind":"query","candidate_ref":"demo-bank@0.0.1/native-balance","inputs":{}},
                {"id":"g_assert","kind":"assert","depends_on":["q_balance"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance != null"}}},
                {"id":"g_branch","kind":"branch","depends_on":["g_assert"],"inputs":{"condition":{"cel":"nodes.q_balance.outputs.balance > 0"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_branch"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
        let context = candidate_context_with_demo_refs();

        let result = validate_segment_write_gates(&segment, &context, None);
        assert!(result.is_ok(), "recursive gate chain should pass: {result:?}");
    }

    #[test]
    fn write_gate_rejects_chain_without_query_backing() {
        let segment: PlanSketchSegment = serde_json::from_value(json!({
            "segment_id": "seg-1",
            "cursor_in": "0",
            "cursor_out": "1",
            "done": false,
            "steps": [
                {"id":"g_assert","kind":"assert","inputs":{"condition":{"cel":"true"}}},
                {"id":"g_branch","kind":"branch","depends_on":["g_assert"],"inputs":{"condition":{"cel":"true"}}},
                {"id":"a_transfer","kind":"action","candidate_ref":"demo-bank@0.0.1/native-transfer","depends_on":["g_branch"],"inputs":{}}
            ],
            "extensions": {}
        }))
        .expect("segment");
        let context = candidate_context_with_demo_refs();

        let error = validate_segment_write_gates(&segment, &context, None)
            .expect_err("missing query-backed gate chain must fail");
        assert_eq!(
            error.pointer("/reason_code").and_then(Value::as_str),
            Some("write_gate_missing")
        );
        assert!(error
            .pointer("/issues/0/reason_code")
            .and_then(Value::as_str)
            .is_some_and(|code| code == "missing_query_assert_branch_chain"));
    }
}
