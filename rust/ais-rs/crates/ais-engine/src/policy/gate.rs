use crate::execution_type::is_core_execution_type;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEnforcementOptions {
    #[serde(default)]
    pub strict_allowlist: bool,
    #[serde(default)]
    pub hard_block_on_missing: bool,
    #[serde(default)]
    pub enforce_plugin_execution_allowlist: bool,
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
    #[serde(default)]
    pub constraint_templates: Vec<PolicyConstraintTemplateRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyConstraintTemplateRef {
    pub name: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyGateReasonCode {
    MissingHardBlockFields,
    MissingFields,
    UnknownFields,
    AllowlistChainNotAllowed,
    AllowlistChainsEmpty,
    AllowlistExecutionTypeNotAllowed,
    AllowlistExecutionTypeUnknown,
    AllowlistActionRefNotAllowed,
    AllowlistActionRefUnknown,
    ThresholdRiskLevelExceeded,
    ThresholdRiskLevelUnknown,
    ConstraintTemplateViolated,
    ConstraintTemplateUnknown,
    ConstraintTemplateInvalidParams,
}

impl PolicyGateReasonCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingHardBlockFields => "missing_hard_block_fields",
            Self::MissingFields => "missing_fields",
            Self::UnknownFields => "unknown_fields",
            Self::AllowlistChainNotAllowed => "allowlist_chain_not_allowed",
            Self::AllowlistChainsEmpty => "allowlist_chains_empty",
            Self::AllowlistExecutionTypeNotAllowed => "allowlist_execution_type_not_allowed",
            Self::AllowlistExecutionTypeUnknown => "allowlist_execution_type_unknown",
            Self::AllowlistActionRefNotAllowed => "allowlist_action_ref_not_allowed",
            Self::AllowlistActionRefUnknown => "allowlist_action_ref_unknown",
            Self::ThresholdRiskLevelExceeded => "threshold_risk_level_exceeded",
            Self::ThresholdRiskLevelUnknown => "threshold_risk_level_unknown",
            Self::ConstraintTemplateViolated => "constraint_template_violated",
            Self::ConstraintTemplateUnknown => "constraint_template_unknown",
            Self::ConstraintTemplateInvalidParams => "constraint_template_invalid_params",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyGateOutput {
    Ok {
        #[serde(default)]
        details: Map<String, Value>,
    },
    NeedUserConfirm {
        reason_code: PolicyGateReasonCode,
        reason: String,
        #[serde(default)]
        details: Map<String, Value>,
    },
    HardBlock {
        reason_code: PolicyGateReasonCode,
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
    let param_roles = read_policy_param_roles(node_object);
    let required_fields = read_policy_required_fields(node_object);
    let constraint_templates = read_policy_constraint_templates(node_object);

    let spend_amount =
        get_string_by_param_role(&params, &param_roles, "spend_amount", "spend_amount");
    let slippage_bps = get_u64_by_param_role(&params, &param_roles, "slippage_bps", "slippage_bps");
    let approval_amount =
        get_string_by_param_role(&params, &param_roles, "approval_amount", "approval_amount");
    let unlimited_approval = params.get("unlimited_approval").and_then(Value::as_bool);
    let spender_address =
        get_string_by_param_role(&params, &param_roles, "spender_address", "spender_address");

    let mut missing_fields = Vec::<String>::new();
    let mut unknown_fields = Vec::<String>::new();
    let mut hard_block_fields = Vec::<String>::new();

    if chain.is_empty() {
        hard_block_fields.push("chain".to_string());
    }
    for field in &required_fields {
        match field.as_str() {
            "spend_amount" if spend_amount.is_none() => {
                missing_fields.push("spend_amount".to_string())
            }
            "slippage_bps" if slippage_bps.is_none() => {
                missing_fields.push("slippage_bps".to_string())
            }
            "approval_amount" if approval_amount.is_none() => {
                missing_fields.push("approval_amount".to_string())
            }
            "spender_address" if spender_address.is_none() => {
                missing_fields.push("spender_address".to_string())
            }
            "unlimited_approval" if unlimited_approval.is_none() => {
                unknown_fields.push("unlimited_approval".to_string())
            }
            _ => {}
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
        constraint_templates,
    }
}

pub fn enforce_policy_gate(
    input: &PolicyGateInput,
    options: &PolicyEnforcementOptions,
) -> PolicyGateOutput {
    if !input.hard_block_fields.is_empty() {
        return PolicyGateOutput::HardBlock {
            reason_code: PolicyGateReasonCode::MissingHardBlockFields,
            reason: reason_message(&PolicyGateReasonCode::MissingHardBlockFields).to_string(),
            details: map_from_entries(vec![(
                "hard_block_fields",
                Value::Array(
                    input
                        .hard_block_fields
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )]),
        };
    }

    if let Some(output) = enforce_constraint_templates(input) {
        return output;
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
                reason_code: PolicyGateReasonCode::MissingFields,
                reason: reason_message(&PolicyGateReasonCode::MissingFields).to_string(),
                details: map_from_entries(vec![(
                    "missing_fields",
                    Value::Array(
                        input
                            .missing_fields
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                )]),
            };
        }
        return PolicyGateOutput::NeedUserConfirm {
            reason_code: PolicyGateReasonCode::MissingFields,
            reason: reason_message(&PolicyGateReasonCode::MissingFields).to_string(),
            details: map_from_entries(vec![(
                "missing_fields",
                Value::Array(
                    input
                        .missing_fields
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )]),
        };
    }

    if !input.unknown_fields.is_empty() {
        return PolicyGateOutput::NeedUserConfirm {
            reason_code: PolicyGateReasonCode::UnknownFields,
            reason: reason_message(&PolicyGateReasonCode::UnknownFields).to_string(),
            details: map_from_entries(vec![(
                "unknown_fields",
                Value::Array(
                    input
                        .unknown_fields
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
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
            reason_code: PolicyGateReasonCode::AllowlistChainNotAllowed,
            reason: reason_message(&PolicyGateReasonCode::AllowlistChainNotAllowed).to_string(),
            details: map_from_entries(vec![
                ("chain", Value::String(input.chain.clone())),
                (
                    "allowlisted_chains",
                    Value::Array(
                        allowlist
                            .chains
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                ),
            ]),
        });
    }

    if strict && allowlist.chains.is_empty() {
        return Some(PolicyGateOutput::HardBlock {
            reason_code: PolicyGateReasonCode::AllowlistChainsEmpty,
            reason: reason_message(&PolicyGateReasonCode::AllowlistChainsEmpty).to_string(),
            details: Map::new(),
        });
    }

    if let Some(execution_type) = &input.execution_type {
        if !is_core_execution_type(execution_type) {
            let enforce = options.enforce_plugin_execution_allowlist;
            let allowlisted = allowlist.execution_types.as_slice();
            let allowed = allowlisted.iter().any(|allowed| allowed == execution_type);
            if enforce && allowlisted.is_empty() {
                return Some(PolicyGateOutput::HardBlock {
                    reason_code: PolicyGateReasonCode::AllowlistExecutionTypeNotAllowed,
                    reason: reason_message(&PolicyGateReasonCode::AllowlistExecutionTypeNotAllowed)
                        .to_string(),
                    details: map_from_entries(vec![
                        ("execution_type", Value::String(execution_type.clone())),
                        ("allowlisted_execution_types", Value::Array(Vec::new())),
                    ]),
                });
            }
            if !allowlisted.is_empty() && !allowed {
                return Some(PolicyGateOutput::HardBlock {
                    reason_code: PolicyGateReasonCode::AllowlistExecutionTypeNotAllowed,
                    reason: reason_message(&PolicyGateReasonCode::AllowlistExecutionTypeNotAllowed)
                        .to_string(),
                    details: map_from_entries(vec![
                        ("execution_type", Value::String(execution_type.clone())),
                        (
                            "allowlisted_execution_types",
                            Value::Array(allowlisted.iter().cloned().map(Value::String).collect()),
                        ),
                    ]),
                });
            }
        }
    } else if options.enforce_plugin_execution_allowlist
        || (strict && !allowlist.execution_types.is_empty())
    {
        return Some(PolicyGateOutput::NeedUserConfirm {
            reason_code: PolicyGateReasonCode::AllowlistExecutionTypeUnknown,
            reason: reason_message(&PolicyGateReasonCode::AllowlistExecutionTypeUnknown)
                .to_string(),
            details: Map::new(),
        });
    }

    if let Some(action_ref) = &input.action_ref {
        if !allowlist.action_refs.is_empty()
            && !allowlist
                .action_refs
                .iter()
                .any(|allowed| allowed == action_ref)
        {
            return Some(PolicyGateOutput::HardBlock {
                reason_code: PolicyGateReasonCode::AllowlistActionRefNotAllowed,
                reason: reason_message(&PolicyGateReasonCode::AllowlistActionRefNotAllowed)
                    .to_string(),
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
            reason_code: PolicyGateReasonCode::AllowlistActionRefUnknown,
            reason: reason_message(&PolicyGateReasonCode::AllowlistActionRefUnknown).to_string(),
            details: Map::new(),
        });
    }

    None
}

fn enforce_constraint_templates(input: &PolicyGateInput) -> Option<PolicyGateOutput> {
    if input.constraint_templates.is_empty() {
        return None;
    }

    let mut matched_constraints = Vec::<Value>::new();
    let mut violations = Vec::<Value>::new();
    let mut has_hard_block = false;

    for template in &input.constraint_templates {
        let evaluation = match evaluate_constraint_template(template, input) {
            Ok(evaluation) => evaluation,
            Err(reason_code) => {
                let reason = reason_message(&reason_code).to_string();
                return Some(PolicyGateOutput::NeedUserConfirm {
                    reason_code,
                    reason,
                    details: map_from_entries(vec![
                        ("template_name", Value::String(template.name.clone())),
                        ("template_params", Value::Object(template.params.clone())),
                    ]),
                });
            }
        };
        let Some((effect, reason_code, message)) = evaluation else {
            continue;
        };
        matched_constraints.push(Value::String(template.name.clone()));
        violations.push(json_violation(
            template.name.as_str(),
            effect,
            reason_code,
            message.as_str(),
        ));
        if matches!(effect, ConstraintEffect::HardBlock) {
            has_hard_block = true;
        }
    }

    if violations.is_empty() {
        return None;
    }

    let details = map_from_entries(vec![
        ("matched_constraints", Value::Array(matched_constraints)),
        ("violations", Value::Array(violations)),
    ]);

    if has_hard_block {
        return Some(PolicyGateOutput::HardBlock {
            reason_code: PolicyGateReasonCode::ConstraintTemplateViolated,
            reason: reason_message(&PolicyGateReasonCode::ConstraintTemplateViolated).to_string(),
            details,
        });
    }

    Some(PolicyGateOutput::NeedUserConfirm {
        reason_code: PolicyGateReasonCode::ConstraintTemplateViolated,
        reason: reason_message(&PolicyGateReasonCode::ConstraintTemplateViolated).to_string(),
        details,
    })
}

fn enforce_thresholds(
    input: &PolicyGateInput,
    options: &PolicyEnforcementOptions,
) -> Option<PolicyGateOutput> {
    let thresholds = &options.thresholds;
    let is_action = input
        .action_ref
        .as_deref()
        .map(|value| value.starts_with("action:"))
        .unwrap_or(true);

    if is_action {
        if let Some(max_risk_level) = thresholds.max_risk_level {
            match input.risk_level {
                Some(risk_level) => {
                    if risk_level > max_risk_level {
                        return Some(PolicyGateOutput::NeedUserConfirm {
                            reason_code: PolicyGateReasonCode::ThresholdRiskLevelExceeded,
                            reason: reason_message(
                                &PolicyGateReasonCode::ThresholdRiskLevelExceeded,
                            )
                            .to_string(),
                            details: map_from_entries(vec![
                                ("risk_level", Value::Number((risk_level as u64).into())),
                                (
                                    "max_risk_level",
                                    Value::Number((max_risk_level as u64).into()),
                                ),
                            ]),
                        });
                    }
                }
                None => {
                    return Some(PolicyGateOutput::NeedUserConfirm {
                        reason_code: PolicyGateReasonCode::ThresholdRiskLevelUnknown,
                        reason: reason_message(&PolicyGateReasonCode::ThresholdRiskLevelUnknown)
                            .to_string(),
                        details: Map::new(),
                    });
                }
            }
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintEffect {
    HardBlock,
    NeedUserConfirm,
}

fn evaluate_constraint_template(
    template: &PolicyConstraintTemplateRef,
    input: &PolicyGateInput,
) -> Result<Option<(ConstraintEffect, String, String)>, PolicyGateReasonCode> {
    match template.name.as_str() {
        "max_spend" => evaluate_max_spend(template, input),
        "max_slippage_bps" => evaluate_max_slippage_bps(template, input),
        "disallow_unlimited_approval" => Ok(match input.unlimited_approval {
            Some(true) => Some((
                ConstraintEffect::HardBlock,
                "constraint_unlimited_approval_disallowed".to_string(),
                "unlimited approval is disallowed by constraint template".to_string(),
            )),
            _ => None,
        }),
        _ => Err(PolicyGateReasonCode::ConstraintTemplateUnknown),
    }
}

fn evaluate_max_spend(
    template: &PolicyConstraintTemplateRef,
    input: &PolicyGateInput,
) -> Result<Option<(ConstraintEffect, String, String)>, PolicyGateReasonCode> {
    let Some(limit) = template_param_u128(&template.params, "amount_atomic") else {
        return Err(PolicyGateReasonCode::ConstraintTemplateInvalidParams);
    };
    let Some(spend_amount) = input.spend_amount.as_deref().and_then(parse_u128) else {
        return Ok(None);
    };
    if spend_amount > limit {
        return Ok(Some((
            ConstraintEffect::NeedUserConfirm,
            "constraint_max_spend_exceeded".to_string(),
            "spend amount exceeds max_spend template limit".to_string(),
        )));
    }
    Ok(None)
}

fn evaluate_max_slippage_bps(
    template: &PolicyConstraintTemplateRef,
    input: &PolicyGateInput,
) -> Result<Option<(ConstraintEffect, String, String)>, PolicyGateReasonCode> {
    let limit = template_param_u64(&template.params, "max_bps")
        .or_else(|| template_param_u64(&template.params, "max_slippage_bps"))
        .ok_or(PolicyGateReasonCode::ConstraintTemplateInvalidParams)?;
    let Some(slippage_bps) = input.slippage_bps else {
        return Ok(None);
    };
    if slippage_bps > limit {
        return Ok(Some((
            ConstraintEffect::NeedUserConfirm,
            "constraint_max_slippage_exceeded".to_string(),
            "slippage exceeds max_slippage_bps template limit".to_string(),
        )));
    }
    Ok(None)
}

fn template_param_u64(params: &Map<String, Value>, key: &str) -> Option<u64> {
    params.get(key).and_then(value_to_u64)
}

fn template_param_u128(params: &Map<String, Value>, key: &str) -> Option<u128> {
    params.get(key).and_then(parse_value_u128)
}

fn parse_value_u128(value: &Value) -> Option<u128> {
    match value {
        Value::String(value) => parse_u128(value.as_str()),
        Value::Number(value) => value.as_u64().map(|v| v as u128),
        _ => None,
    }
}

fn parse_u128(value: &str) -> Option<u128> {
    value.trim().parse::<u128>().ok()
}

fn json_violation(
    name: &str,
    effect: ConstraintEffect,
    reason_code: String,
    message: &str,
) -> Value {
    serde_json::json!({
        "name": name,
        "effect": match effect {
            ConstraintEffect::HardBlock => "hard_block",
            ConstraintEffect::NeedUserConfirm => "need_user_confirm",
        },
        "reason_code": reason_code,
        "message": message,
    })
}

fn get_string_by_param_role(
    params: &Map<String, Value>,
    roles: &BTreeMap<String, String>,
    role: &str,
    canonical_key: &str,
) -> Option<String> {
    if let Some(key) = roles.get(role) {
        return params.get(key.as_str()).and_then(value_to_string);
    }
    params.get(canonical_key).and_then(value_to_string)
}

fn get_u64_by_param_role(
    params: &Map<String, Value>,
    roles: &BTreeMap<String, String>,
    role: &str,
    canonical_key: &str,
) -> Option<u64> {
    if let Some(key) = roles.get(role) {
        return params.get(key.as_str()).and_then(value_to_u64);
    }
    params.get(canonical_key).and_then(value_to_u64)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn value_to_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn read_policy_param_roles(
    node_object: Option<&serde_json::Map<String, Value>>,
) -> BTreeMap<String, String> {
    let Some(node_object) = node_object else {
        return BTreeMap::new();
    };
    let roles = node_object
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("policy"))
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("param_roles"))
        .and_then(Value::as_object);
    let Some(roles) = roles else {
        return BTreeMap::new();
    };
    roles
        .iter()
        .filter_map(|(role, value)| {
            value
                .as_str()
                .map(|value| (role.clone(), value.to_string()))
        })
        .collect::<BTreeMap<_, _>>()
}

fn read_policy_required_fields(
    node_object: Option<&serde_json::Map<String, Value>>,
) -> Vec<String> {
    let Some(node_object) = node_object else {
        return Vec::new();
    };
    let fields = node_object
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("policy"))
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("required_fields"))
        .and_then(Value::as_array);
    let Some(fields) = fields else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .filter(|field| !field.trim().is_empty())
        .collect()
}

fn read_policy_constraint_templates(
    node_object: Option<&serde_json::Map<String, Value>>,
) -> Vec<PolicyConstraintTemplateRef> {
    let Some(node_object) = node_object else {
        return Vec::new();
    };
    let Some(templates) = node_object
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get("policy"))
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("constraint_templates"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    templates
        .iter()
        .filter_map(|item| {
            let item_obj = item.as_object()?;
            let name = item_obj
                .get("name")
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            if name.is_empty() {
                return None;
            }
            let params = item_obj
                .get("params")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Some(PolicyConstraintTemplateRef { name, params })
        })
        .collect()
}

fn map_from_entries(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in entries {
        out.insert(key.to_string(), value);
    }
    out
}

fn dedup_sort(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn reason_message(code: &PolicyGateReasonCode) -> &'static str {
    match code {
        PolicyGateReasonCode::MissingHardBlockFields => "policy gate required fields are missing",
        PolicyGateReasonCode::MissingFields => "policy gate input is incomplete",
        PolicyGateReasonCode::UnknownFields => "policy gate input has unknown fields",
        PolicyGateReasonCode::AllowlistChainNotAllowed => "chain is not allowlisted by pack",
        PolicyGateReasonCode::AllowlistChainsEmpty => "chain allowlist is empty",
        PolicyGateReasonCode::AllowlistExecutionTypeNotAllowed => {
            "execution type is not allowlisted by pack"
        }
        PolicyGateReasonCode::AllowlistExecutionTypeUnknown => {
            "execution type is unknown for allowlist check"
        }
        PolicyGateReasonCode::AllowlistActionRefNotAllowed => {
            "action ref is not allowlisted by pack"
        }
        PolicyGateReasonCode::AllowlistActionRefUnknown => {
            "action ref is unknown for allowlist check"
        }
        PolicyGateReasonCode::ThresholdRiskLevelExceeded => {
            "risk level exceeds confirmation threshold"
        }
        PolicyGateReasonCode::ThresholdRiskLevelUnknown => {
            "risk level is unknown for threshold check"
        }
        PolicyGateReasonCode::ConstraintTemplateViolated => {
            "constraint template policy requires review"
        }
        PolicyGateReasonCode::ConstraintTemplateUnknown => "constraint template is unknown",
        PolicyGateReasonCode::ConstraintTemplateInvalidParams => {
            "constraint template parameters are invalid"
        }
    }
}

#[cfg(test)]
#[path = "gate_test.rs"]
mod tests;
