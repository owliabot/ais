use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyPackAllowlist {
    #[serde(default)]
    pub chains: Vec<String>,
    #[serde(default)]
    pub execution_types: Vec<String>,
    #[serde(default)]
    pub action_refs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyThresholdRules {
    #[serde(default)]
    pub max_risk_level: Option<u8>,
    #[serde(default)]
    pub max_spend_amount: Option<String>,
    #[serde(default)]
    pub max_slippage_bps: Option<u64>,
    #[serde(default)]
    pub forbid_unlimited_approval: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEnforcementOptions {
    #[serde(default)]
    pub strict_allowlist: bool,
    #[serde(default)]
    pub hard_block_on_missing: bool,
    #[serde(default)]
    pub allowlist: PolicyPackAllowlist,
    #[serde(default)]
    pub thresholds: PolicyThresholdRules,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyGateInput {
    #[serde(default)]
    pub node_id: Option<String>,
    pub chain: String,
    #[serde(default)]
    pub execution_type: Option<String>,
    #[serde(default)]
    pub action_ref: Option<String>,
    #[serde(default)]
    pub risk_level: Option<u8>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub spend_amount: Option<String>,
    #[serde(default)]
    pub slippage_bps: Option<u64>,
    #[serde(default)]
    pub approval_amount: Option<String>,
    #[serde(default)]
    pub unlimited_approval: Option<bool>,
    #[serde(default)]
    pub spender_address: Option<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub unknown_fields: Vec<String>,
    #[serde(default)]
    pub hard_block_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyGateOutput {
    Ok {
        #[serde(default)]
        details: Map<String, Value>,
    },
    NeedUserConfirm {
        reason: String,
        #[serde(default)]
        details: Map<String, Value>,
    },
    HardBlock {
        reason: String,
        #[serde(default)]
        details: Map<String, Value>,
    },
}

pub fn extract_policy_gate_input(
    node: &Value,
    resolved_params: Option<&Map<String, Value>>,
    action_ref: Option<String>,
    risk_level: Option<u8>,
    risk_tags: Vec<String>,
) -> PolicyGateInput {
    let node_object = node.as_object();
    let node_id = node_object
        .and_then(|object| object.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let chain = node_object
        .and_then(|object| object.get("chain"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let execution_type = node_object
        .and_then(|object| object.get("execution"))
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("type"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let params = resolved_params.cloned().unwrap_or_default();
    let spend_amount = get_first_string(
        &params,
        &["spend_amount", "amount_in", "amount", "input_amount"],
    );
    let slippage_bps = get_first_u64(&params, &["slippage_bps", "max_slippage_bps"]);
    let approval_amount = get_first_string(&params, &["approval_amount", "max_approval"]);
    let unlimited_approval = params
        .get("unlimited_approval")
        .and_then(Value::as_bool);
    let spender_address = get_first_string(&params, &["spender", "spender_address", "delegate"]);

    let mut missing_fields = Vec::<String>::new();
    let mut unknown_fields = Vec::<String>::new();
    let mut hard_block_fields = Vec::<String>::new();

    if chain.is_empty() {
        hard_block_fields.push("chain".to_string());
    }

    let method_or_instruction = node_object
        .and_then(|object| object.get("execution"))
        .and_then(Value::as_object)
        .and_then(|execution| {
            execution
                .get("method")
                .or_else(|| execution.get("instruction"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_lowercase();
    let is_swap_like = method_or_instruction.contains("swap");
    let is_approve_like = method_or_instruction.contains("approve");

    if is_swap_like {
        if spend_amount.is_none() {
            missing_fields.push("spend_amount".to_string());
        }
        if slippage_bps.is_none() {
            missing_fields.push("slippage_bps".to_string());
        }
    }
    if is_approve_like {
        if approval_amount.is_none() {
            missing_fields.push("approval_amount".to_string());
        }
        if spender_address.is_none() {
            missing_fields.push("spender_address".to_string());
        }
        if unlimited_approval.is_none() {
            unknown_fields.push("unlimited_approval".to_string());
        }
    }

    missing_fields = dedup_sort(missing_fields);
    unknown_fields = dedup_sort(unknown_fields);
    hard_block_fields = dedup_sort(hard_block_fields);

    PolicyGateInput {
        node_id,
        chain,
        execution_type,
        action_ref,
        risk_level,
        risk_tags,
        spend_amount,
        slippage_bps,
        approval_amount,
        unlimited_approval,
        spender_address,
        missing_fields,
        unknown_fields,
        hard_block_fields,
    }
}

pub fn enforce_policy_gate(
    input: &PolicyGateInput,
    options: &PolicyEnforcementOptions,
) -> PolicyGateOutput {
    if !input.hard_block_fields.is_empty() {
        return PolicyGateOutput::HardBlock {
            reason: "policy gate required fields are missing".to_string(),
            details: map_from_entries(vec![(
                "hard_block_fields",
                Value::Array(input.hard_block_fields.iter().cloned().map(Value::String).collect()),
            )]),
        };
    }

    if let Some(output) = enforce_allowlist(input, options) {
        return output;
    }
    if let Some(output) = enforce_thresholds(input, options) {
        return output;
    }

    if !input.missing_fields.is_empty() {
        if options.hard_block_on_missing {
            return PolicyGateOutput::HardBlock {
                reason: "policy gate input is incomplete".to_string(),
                details: map_from_entries(vec![(
                    "missing_fields",
                    Value::Array(input.missing_fields.iter().cloned().map(Value::String).collect()),
                )]),
            };
        }
        return PolicyGateOutput::NeedUserConfirm {
            reason: "policy gate input is incomplete".to_string(),
            details: map_from_entries(vec![(
                "missing_fields",
                Value::Array(input.missing_fields.iter().cloned().map(Value::String).collect()),
            )]),
        };
    }

    if !input.unknown_fields.is_empty() {
        return PolicyGateOutput::NeedUserConfirm {
            reason: "policy gate input has unknown fields".to_string(),
            details: map_from_entries(vec![(
                "unknown_fields",
                Value::Array(input.unknown_fields.iter().cloned().map(Value::String).collect()),
            )]),
        };
    }

    PolicyGateOutput::Ok {
        details: Map::new(),
    }
}

fn enforce_allowlist(
    input: &PolicyGateInput,
    options: &PolicyEnforcementOptions,
) -> Option<PolicyGateOutput> {
    let allowlist = &options.allowlist;
    let strict = options.strict_allowlist;

    if !allowlist.chains.is_empty() && !allowlist.chains.iter().any(|chain| chain == &input.chain) {
        return Some(PolicyGateOutput::HardBlock {
            reason: "chain is not allowlisted by pack".to_string(),
            details: map_from_entries(vec![
                ("chain", Value::String(input.chain.clone())),
                (
                    "allowlisted_chains",
                    Value::Array(allowlist.chains.iter().cloned().map(Value::String).collect()),
                ),
            ]),
        });
    }

    if strict && allowlist.chains.is_empty() {
        return Some(PolicyGateOutput::HardBlock {
            reason: "chain allowlist is empty".to_string(),
            details: Map::new(),
        });
    }

    if let Some(execution_type) = &input.execution_type {
        if !allowlist.execution_types.is_empty()
            && !allowlist
                .execution_types
                .iter()
                .any(|allowed| allowed == execution_type)
        {
            return Some(PolicyGateOutput::HardBlock {
                reason: "execution type is not allowlisted by pack".to_string(),
                details: map_from_entries(vec![
                    ("execution_type", Value::String(execution_type.clone())),
                    (
                        "allowlisted_execution_types",
                        Value::Array(
                            allowlist
                                .execution_types
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect(),
                        ),
                    ),
                ]),
            });
        }
    } else if strict && !allowlist.execution_types.is_empty() {
        return Some(PolicyGateOutput::NeedUserConfirm {
            reason: "execution type is unknown for allowlist check".to_string(),
            details: Map::new(),
        });
    }

    if let Some(action_ref) = &input.action_ref {
        if !allowlist.action_refs.is_empty()
            && !allowlist.action_refs.iter().any(|allowed| allowed == action_ref)
        {
            return Some(PolicyGateOutput::HardBlock {
                reason: "action ref is not allowlisted by pack".to_string(),
                details: map_from_entries(vec![
                    ("action_ref", Value::String(action_ref.clone())),
                    (
                        "allowlisted_action_refs",
                        Value::Array(
                            allowlist
                                .action_refs
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect(),
                        ),
                    ),
                ]),
            });
        }
    } else if strict && !allowlist.action_refs.is_empty() {
        return Some(PolicyGateOutput::NeedUserConfirm {
            reason: "action ref is unknown for allowlist check".to_string(),
            details: Map::new(),
        });
    }

    None
}

fn enforce_thresholds(
    input: &PolicyGateInput,
    options: &PolicyEnforcementOptions,
) -> Option<PolicyGateOutput> {
    let thresholds = &options.thresholds;

    if let (Some(risk_level), Some(max_risk_level)) = (input.risk_level, thresholds.max_risk_level) {
        if risk_level > max_risk_level {
            return Some(PolicyGateOutput::NeedUserConfirm {
                reason: "risk level exceeds confirmation threshold".to_string(),
                details: map_from_entries(vec![
                    ("risk_level", Value::Number((risk_level as u64).into())),
                    ("max_risk_level", Value::Number((max_risk_level as u64).into())),
                ]),
            });
        }
    }

    if let (Some(spend_amount), Some(max_spend_amount)) = (
        input.spend_amount.as_deref(),
        thresholds.max_spend_amount.as_deref(),
    ) {
        if let (Some(spend), Some(max)) = (parse_u128(spend_amount), parse_u128(max_spend_amount)) {
            if spend > max {
                return Some(PolicyGateOutput::HardBlock {
                    reason: "spend amount exceeds hard limit".to_string(),
                    details: map_from_entries(vec![
                        ("spend_amount", Value::String(spend_amount.to_string())),
                        ("max_spend_amount", Value::String(max_spend_amount.to_string())),
                    ]),
                });
            }
        }
    }

    if let (Some(slippage_bps), Some(max_slippage_bps)) =
        (input.slippage_bps, thresholds.max_slippage_bps)
    {
        if slippage_bps > max_slippage_bps {
            return Some(PolicyGateOutput::HardBlock {
                reason: "slippage exceeds hard limit".to_string(),
                details: map_from_entries(vec![
                    ("slippage_bps", Value::Number(slippage_bps.into())),
                    ("max_slippage_bps", Value::Number(max_slippage_bps.into())),
                ]),
            });
        }
    }

    if thresholds.forbid_unlimited_approval && input.unlimited_approval == Some(true) {
        return Some(PolicyGateOutput::HardBlock {
            reason: "unlimited approval is forbidden".to_string(),
            details: Map::new(),
        });
    }

    None
}

fn get_first_string(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| map.get(*key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn get_first_u64(map: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| map.get(*key))
        .and_then(Value::as_u64)
}

fn parse_u128(value: &str) -> Option<u128> {
    value.trim().parse::<u128>().ok()
}

fn map_from_entries(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in entries {
        out.insert(key.to_string(), value);
    }
    out
}

fn dedup_sort(values: Vec<String>) -> Vec<String> {
    values.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

#[cfg(test)]
#[path = "gate_test.rs"]
mod tests;
