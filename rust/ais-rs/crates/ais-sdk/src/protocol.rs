use crate::documents::{PackDocument, ProtocolDocument};
use ais_core::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedDeployment {
    pub chain: String,
    pub deployment: Value,
    pub contracts: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedOperationKind {
    Action,
    Query,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPackOperation {
    pub pack_key: String,
    pub operation_selector: String,
    pub global_constraints: Vec<Value>,
    pub matched_action_rule_ids: Vec<String>,
    pub action_rule_constraints: Vec<Value>,
    pub action_constraints: Vec<Value>,
    pub effective_constraints: Vec<Value>,
    pub action_override: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedOperationSpec {
    pub protocol_ref: String,
    pub protocol_id: String,
    pub operation_key: String,
    pub operation_kind: ResolvedOperationKind,
    pub chain: String,
    pub merged_spec: Value,
    pub deployment: ResolvedDeployment,
    pub pack: Option<ResolvedPackOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenResolutionPolicy {
    pub allow_symbol_input: bool,
    pub require_user_confirm_asset_address: bool,
    pub require_allowlist_for_symbol_resolution: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTokenCandidate {
    pub chain: String,
    pub symbol: String,
    pub address: String,
    pub decimals: Option<u64>,
    pub name: Option<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub allowlisted: bool,
    pub require_user_confirm_asset_address: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenResolutionErrorCode {
    SymbolInputNotAllowed,
    SymbolNotAllowlisted,
    SymbolUnknownForChain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResolutionError {
    pub code: TokenResolutionErrorCode,
    pub chain: String,
    pub symbol: String,
    pub message: String,
}

pub fn resolve_deployment_for_chain(
    protocol: &ProtocolDocument,
    chain: &str,
) -> Option<ResolvedDeployment> {
    let mut best_match = None::<(u8, ResolvedDeployment)>;

    for deployment in &protocol.deployments {
        let Some(pattern) = deployment
            .as_object()
            .and_then(|object| object.get("chain"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(score) = deployment_match_score(pattern, chain) else {
            continue;
        };
        if best_match
            .as_ref()
            .is_some_and(|(best_score, _)| *best_score >= score)
        {
            continue;
        }
        let Some(deployment_object) = deployment.as_object() else {
            continue;
        };
        let contracts = match deployment_object.get("contracts") {
            Some(Value::Object(contracts)) => contracts.clone(),
            Some(_) => continue,
            None => Map::new(),
        };
        best_match = Some((
            score,
            ResolvedDeployment {
                chain: pattern.to_string(),
                deployment: deployment.clone(),
                contracts,
            },
        ));
    }

    best_match.map(|(_, deployment)| deployment)
}

pub fn build_protocol_extension(protocol_ref: &str, deployment: &ResolvedDeployment) -> Value {
    Value::Object(Map::from_iter([
        ("ref".to_string(), Value::String(protocol_ref.to_string())),
        (
            "deployment_chain".to_string(),
            Value::String(deployment.chain.clone()),
        ),
        (
            "contracts".to_string(),
            Value::Object(deployment.contracts.clone()),
        ),
        ("deployment".to_string(), deployment.deployment.clone()),
    ]))
}

pub fn pack_document_hash(pack: &PackDocument) -> Option<String> {
    let value = serde_json::to_value(pack).ok()?;
    stable_hash_hex(&value, &StableJsonOptions::default()).ok()
}

pub fn annotate_composite_step_protocol_bindings(
    execution: &Value,
    protocol_ref: &str,
    protocol: &ProtocolDocument,
    parent_chain: &str,
) -> Result<Value, String> {
    let Some(execution_obj) = execution.as_object() else {
        return Ok(execution.clone());
    };
    if execution_obj.get("type").and_then(Value::as_str) != Some("composite") {
        return Ok(execution.clone());
    }

    let mut annotated = execution_obj.clone();
    let Some(steps) = annotated.get_mut("steps").and_then(Value::as_array_mut) else {
        return Err("composite execution must define steps[]".to_string());
    };

    for (index, step) in steps.iter_mut().enumerate() {
        let Some(step_obj) = step.as_object_mut() else {
            return Err(format!("composite step[{index}] must be an object"));
        };
        let step_id = step_obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("composite step[{index}] missing id"))?
            .to_string();
        let step_chain = step_obj
            .get("chain")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(parent_chain);
        let deployment = resolve_deployment_for_chain(protocol, step_chain).ok_or_else(|| {
            format!("composite step `{step_id}` has no deployment mapping for chain `{step_chain}`")
        })?;
        step_obj.insert("chain".to_string(), Value::String(step_chain.to_string()));
        let extensions = step_obj
            .entry("extensions".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(extensions_obj) = extensions.as_object_mut() else {
            return Err(format!(
                "composite step `{step_id}` has invalid `extensions` object"
            ));
        };
        extensions_obj.insert(
            "protocol".to_string(),
            build_protocol_extension(protocol_ref, &deployment),
        );
    }

    Ok(Value::Object(annotated))
}

pub fn token_resolution_policy(pack: Option<&PackDocument>) -> TokenResolutionPolicy {
    let resolution = pack
        .and_then(|pack| pack.token_policy.as_ref())
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("resolution"))
        .and_then(Value::as_object);

    TokenResolutionPolicy {
        allow_symbol_input: resolution
            .and_then(|resolution| resolution.get("allow_symbol_input"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        require_user_confirm_asset_address: resolution
            .and_then(|resolution| resolution.get("require_user_confirm_asset_address"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        require_allowlist_for_symbol_resolution: resolution
            .and_then(|resolution| resolution.get("require_allowlist_for_symbol_resolution"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

pub fn resolve_token_candidate_for_symbol(
    pack: Option<&PackDocument>,
    protocol: Option<&ProtocolDocument>,
    chain: &str,
    symbol: &str,
) -> Result<ResolvedTokenCandidate, TokenResolutionError> {
    let symbol = symbol.trim();
    let policy = token_resolution_policy(pack);
    if !policy.allow_symbol_input {
        return Err(TokenResolutionError {
            code: TokenResolutionErrorCode::SymbolInputNotAllowed,
            chain: chain.to_string(),
            symbol: symbol.to_string(),
            message: format!("symbol input `{symbol}` is not allowed by pack token_policy"),
        });
    }

    if let Some(candidate) = pack
        .and_then(|pack| {
            pack_token_allowlist_entries(pack)
                .into_iter()
                .find(|entry| entry.chain == chain && entry.symbol.eq_ignore_ascii_case(symbol))
        })
        .map(|entry| ResolvedTokenCandidate {
            chain: entry.chain,
            symbol: entry.symbol,
            address: entry.address,
            decimals: entry.decimals,
            name: entry.name,
            tags: entry.tags,
            source: "pack_allowlist".to_string(),
            allowlisted: true,
            require_user_confirm_asset_address: policy.require_user_confirm_asset_address,
        })
    {
        return Ok(candidate);
    }

    if policy.require_allowlist_for_symbol_resolution {
        return Err(TokenResolutionError {
            code: TokenResolutionErrorCode::SymbolNotAllowlisted,
            chain: chain.to_string(),
            symbol: symbol.to_string(),
            message: format!("token symbol `{symbol}` is not allowlisted for chain `{chain}`"),
        });
    }

    if let Some(candidate) = protocol
        .and_then(|protocol| {
            protocol_supported_asset_entries(protocol)
                .into_iter()
                .find(|entry| entry.chain == chain && entry.symbol.eq_ignore_ascii_case(symbol))
        })
        .map(|entry| ResolvedTokenCandidate {
            chain: entry.chain,
            symbol: entry.symbol,
            address: entry.address,
            decimals: entry.decimals,
            name: entry.name,
            tags: entry.tags,
            source: "protocol_supported_assets".to_string(),
            allowlisted: false,
            require_user_confirm_asset_address: policy.require_user_confirm_asset_address,
        })
    {
        return Ok(candidate);
    }

    Err(TokenResolutionError {
        code: TokenResolutionErrorCode::SymbolUnknownForChain,
        chain: chain.to_string(),
        symbol: symbol.to_string(),
        message: format!("token symbol `{symbol}` could not be resolved for chain `{chain}`"),
    })
}

pub fn resolve_token_candidate_for_address(
    pack: Option<&PackDocument>,
    protocol: Option<&ProtocolDocument>,
    chain: &str,
    address: &str,
) -> Option<ResolvedTokenCandidate> {
    let address = address.trim();
    let policy = token_resolution_policy(pack);
    if let Some(entry) = pack.and_then(|pack| {
        pack_token_allowlist_entries(pack)
            .into_iter()
            .find(|entry| entry.chain == chain && entry.address.eq_ignore_ascii_case(address))
    }) {
        return Some(ResolvedTokenCandidate {
            chain: entry.chain,
            symbol: entry.symbol,
            address: entry.address,
            decimals: entry.decimals,
            name: entry.name,
            tags: entry.tags,
            source: "pack_allowlist".to_string(),
            allowlisted: true,
            require_user_confirm_asset_address: policy.require_user_confirm_asset_address,
        });
    }
    protocol
        .and_then(|protocol| {
            protocol_supported_asset_entries(protocol)
                .into_iter()
                .find(|entry| entry.chain == chain && entry.address.eq_ignore_ascii_case(address))
        })
        .map(|entry| ResolvedTokenCandidate {
            chain: entry.chain,
            symbol: entry.symbol,
            address: entry.address,
            decimals: entry.decimals,
            name: entry.name,
            tags: entry.tags,
            source: "protocol_supported_assets".to_string(),
            allowlisted: false,
            require_user_confirm_asset_address: policy.require_user_confirm_asset_address,
        })
}

pub fn resolve_operation_spec(
    protocol_ref: &str,
    protocol: &ProtocolDocument,
    operation_key: &str,
    operation_kind: ResolvedOperationKind,
    chain: &str,
    pack: Option<&PackDocument>,
) -> Option<ResolvedOperationSpec> {
    let operation_spec = match operation_kind {
        ResolvedOperationKind::Action => protocol.actions.get(operation_key)?,
        ResolvedOperationKind::Query => protocol.queries.get(operation_key)?,
    };
    let deployment = resolve_deployment_for_chain(protocol, chain)?;
    let protocol_id = protocol_id(protocol);
    let pack_resolution = pack.and_then(|pack| {
        resolve_pack_operation(
            pack,
            protocol_id.as_str(),
            operation_key,
            operation_kind.clone(),
        )
    });
    let mut merged_spec = operation_spec.clone();
    if let Some(pack_resolution) = &pack_resolution {
        if let Some(action_override) = &pack_resolution.action_override {
            deep_merge_value(&mut merged_spec, action_override);
        }
    }

    Some(ResolvedOperationSpec {
        protocol_ref: protocol_ref.to_string(),
        protocol_id,
        operation_key: operation_key.to_string(),
        operation_kind,
        chain: chain.to_string(),
        merged_spec,
        deployment,
        pack: pack_resolution,
    })
}

pub fn build_pack_extension(pack: &ResolvedPackOperation) -> Value {
    Value::Object(Map::from_iter([
        ("ref".to_string(), Value::String(pack.pack_key.clone())),
        (
            "operation_selector".to_string(),
            Value::String(pack.operation_selector.clone()),
        ),
        (
            "matched_action_rule_ids".to_string(),
            Value::Array(
                pack.matched_action_rule_ids
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        ),
        (
            "global_constraints".to_string(),
            Value::Array(pack.global_constraints.clone()),
        ),
        (
            "action_rule_constraints".to_string(),
            Value::Array(pack.action_rule_constraints.clone()),
        ),
        (
            "action_constraints".to_string(),
            Value::Array(pack.action_constraints.clone()),
        ),
        (
            "effective_constraints".to_string(),
            Value::Array(pack.effective_constraints.clone()),
        ),
    ]))
}

pub fn build_operation_extension(spec: &ResolvedOperationSpec) -> Value {
    let mut extension = Map::from_iter([
        (
            "protocol_ref".to_string(),
            Value::String(spec.protocol_ref.clone()),
        ),
        (
            "protocol_id".to_string(),
            Value::String(spec.protocol_id.clone()),
        ),
        (
            "kind".to_string(),
            Value::String(operation_kind_label(&spec.operation_kind).to_string()),
        ),
        ("key".to_string(), Value::String(spec.operation_key.clone())),
        (
            "selector".to_string(),
            Value::String(operation_selector(
                spec.protocol_id.as_str(),
                spec.operation_key.as_str(),
            )),
        ),
        (
            "target_chain".to_string(),
            Value::String(spec.chain.clone()),
        ),
        (
            "ref".to_string(),
            Value::String(format!("{}/{}", spec.protocol_ref, spec.operation_key)),
        ),
    ]);
    if let Some(requires_queries) = spec
        .merged_spec
        .as_object()
        .and_then(|spec| spec.get("requires_queries"))
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
    {
        extension.insert(
            "requires_queries".to_string(),
            Value::Array(requires_queries.clone()),
        );
    }
    Value::Object(extension)
}

pub fn build_policy_extension(spec: &ResolvedOperationSpec) -> Value {
    let mut policy = Map::<String, Value>::new();
    copy_policy_gate_metadata_from_operation_spec(&spec.merged_spec, &mut policy);

    if let Some(pack) = &spec.pack {
        if !pack.global_constraints.is_empty() {
            policy.insert(
                "global_constraints".to_string(),
                Value::Array(pack.global_constraints.clone()),
            );
        }
        if !pack.action_rule_constraints.is_empty() {
            policy.insert(
                "action_rule_constraints".to_string(),
                Value::Array(pack.action_rule_constraints.clone()),
            );
        }
        if !pack.action_constraints.is_empty() {
            policy.insert(
                "action_constraints".to_string(),
                Value::Array(pack.action_constraints.clone()),
            );
        }
        if !pack.effective_constraints.is_empty() {
            policy.insert(
                "effective_constraints".to_string(),
                Value::Array(pack.effective_constraints.clone()),
            );
        }
    }

    Value::Object(policy)
}

fn copy_policy_gate_metadata_from_operation_spec(
    operation_spec: &Value,
    policy: &mut Map<String, Value>,
) {
    let Some(obj) = operation_spec.as_object() else {
        return;
    };
    let Some(params) = obj.get("params").and_then(Value::as_array) else {
        return;
    };

    let mut role_to_param = Map::<String, Value>::new();

    let slippage_param = find_param_by_name(params, "slippage_bps");
    if let Some(name) = slippage_param {
        role_to_param.insert("slippage_bps".to_string(), Value::String(name));
    }

    if let Some(name) = find_param_by_name(params, "spend_amount") {
        role_to_param.insert("spend_amount".to_string(), Value::String(name));
    }
    if let Some(name) = find_param_by_name(params, "amount_atomic") {
        role_to_param.insert("spend_amount".to_string(), Value::String(name));
    }
    if let Some(name) = find_param_by_name(params, "approval_amount") {
        role_to_param.insert("approval_amount".to_string(), Value::String(name));
    }
    if let Some(name) = find_param_by_name(params, "spender_address") {
        role_to_param.insert("spender_address".to_string(), Value::String(name));
    }
    if let Some(name) = find_param_by_name(params, "unlimited_approval") {
        role_to_param.insert("unlimited_approval".to_string(), Value::String(name));
    }

    let risk_tags = obj
        .get("risk_tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let is_approval_related = risk_tags.iter().any(|tag| tag == "approval");
    if is_approval_related {
        if let Some(name) = find_param_by_name(params, "spender") {
            role_to_param.insert("spender_address".to_string(), Value::String(name));
        }
        if let Some(name) = find_param_by_name(params, "amount") {
            role_to_param.insert("approval_amount".to_string(), Value::String(name));
        }
    }

    if role_to_param.is_empty() {
        return;
    }

    let required_fields = role_to_param.keys().cloned().collect::<Vec<_>>();
    policy.insert("param_roles".to_string(), Value::Object(role_to_param));
    policy.insert(
        "required_fields".to_string(),
        Value::Array(required_fields.into_iter().map(Value::String).collect()),
    );
}

fn find_param_by_name(params: &[Value], expected: &str) -> Option<String> {
    params
        .iter()
        .filter_map(Value::as_object)
        .find(|param| param.get("name").and_then(Value::as_str) == Some(expected))
        .and_then(|param| param.get("name").and_then(Value::as_str))
        .map(str::to_string)
}

fn deployment_match_score(pattern: &str, chain: &str) -> Option<u8> {
    if pattern == chain {
        return Some(3);
    }
    if pattern == "*" {
        return Some(1);
    }
    let (pattern_namespace, pattern_reference) = pattern.split_once(':')?;
    let (chain_namespace, _) = chain.split_once(':')?;
    if pattern_namespace == chain_namespace && pattern_reference == "*" {
        return Some(2);
    }
    None
}

fn resolve_pack_operation(
    pack: &PackDocument,
    protocol_id: &str,
    operation_key: &str,
    operation_kind: ResolvedOperationKind,
) -> Option<ResolvedPackOperation> {
    let operation_selector = operation_selector(protocol_id, operation_key);
    let overrides = pack.overrides.as_ref().and_then(Value::as_object);
    let action_override = overrides
        .and_then(|overrides| overrides.get("actions"))
        .and_then(Value::as_object)
        .and_then(|actions| actions.get(operation_selector.as_str()))
        .cloned();

    let mut matched_action_rule_ids = Vec::<String>::new();
    let mut action_rule_constraints = Vec::<Value>::new();
    if matches!(operation_kind, ResolvedOperationKind::Action) {
        if let Some(action_rules) = overrides
            .and_then(|overrides| overrides.get("action_rules"))
            .and_then(Value::as_array)
        {
            for (index, rule) in action_rules.iter().enumerate() {
                let Some(rule_object) = rule.as_object() else {
                    continue;
                };
                let matches_action = rule_object
                    .get("actions")
                    .and_then(Value::as_array)
                    .is_some_and(|actions| {
                        actions
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|action| action == operation_selector)
                    });
                if !matches_action {
                    continue;
                }
                let rule_id = rule_object
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("action_rule_{index}"));
                matched_action_rule_ids.push(rule_id);
                if let Some(constraints) = rule_object.get("constraints").and_then(Value::as_array)
                {
                    action_rule_constraints.extend(constraints.clone());
                }
            }
        }
    }

    let global_constraints = pack
        .policy
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("constraints"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let action_constraints = action_override
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|override_obj| override_obj.get("constraints"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut effective_constraints = Vec::<Value>::new();
    effective_constraints.extend(global_constraints.clone());
    effective_constraints.extend(action_rule_constraints.clone());
    effective_constraints.extend(action_constraints.clone());

    if global_constraints.is_empty()
        && matched_action_rule_ids.is_empty()
        && action_constraints.is_empty()
        && action_override.is_none()
    {
        return None;
    }

    Some(ResolvedPackOperation {
        pack_key: pack_identity(pack),
        operation_selector,
        global_constraints,
        matched_action_rule_ids,
        action_rule_constraints,
        action_constraints,
        effective_constraints,
        action_override,
    })
}

#[derive(Debug, Clone)]
struct TokenRegistryEntry {
    chain: String,
    symbol: String,
    address: String,
    decimals: Option<u64>,
    name: Option<String>,
    tags: Vec<String>,
}

fn pack_token_allowlist_entries(pack: &PackDocument) -> Vec<TokenRegistryEntry> {
    pack.token_policy
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|policy| policy.get("allowlist"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(token_registry_entry_from_allowlist)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn protocol_supported_asset_entries(protocol: &ProtocolDocument) -> Vec<TokenRegistryEntry> {
    let mut out = Vec::<TokenRegistryEntry>::new();
    for asset in &protocol.supported_assets {
        let Some(asset_obj) = asset.as_object() else {
            continue;
        };
        let Some(symbol) = asset_obj
            .get("symbol")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
        else {
            continue;
        };
        let name = asset_obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tags = asset_obj
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(deployments) = asset_obj.get("deployments").and_then(Value::as_array) {
            for deployment in deployments {
                let Some(deployment_obj) = deployment.as_object() else {
                    continue;
                };
                let Some(chain) = deployment_obj
                    .get("chain")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                let Some(address) = deployment_obj
                    .get("address")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                let decimals = deployment_obj.get("decimals").and_then(value_to_u64);
                out.push(TokenRegistryEntry {
                    chain: chain.to_string(),
                    symbol: symbol.to_string(),
                    address: address.to_string(),
                    decimals,
                    name: name.clone(),
                    tags: tags.clone(),
                });
            }
            continue;
        }
        if let Some(addresses) = asset_obj.get("addresses").and_then(Value::as_object) {
            let decimals_by_chain = asset_obj.get("decimals").and_then(Value::as_object);
            for (chain, address_value) in addresses {
                let Some(address) = address_value.as_str().map(str::trim) else {
                    continue;
                };
                if address.is_empty() {
                    continue;
                }
                let decimals = decimals_by_chain
                    .and_then(|items| items.get(chain.as_str()))
                    .and_then(value_to_u64);
                out.push(TokenRegistryEntry {
                    chain: chain.clone(),
                    symbol: symbol.to_string(),
                    address: address.to_string(),
                    decimals,
                    name: name.clone(),
                    tags: tags.clone(),
                });
            }
        }
    }
    out
}

fn token_registry_entry_from_allowlist(value: &Value) -> Option<TokenRegistryEntry> {
    let obj = value.as_object()?;
    let chain = obj.get("chain").and_then(Value::as_str)?.trim();
    let symbol = obj.get("symbol").and_then(Value::as_str)?.trim();
    let address = obj.get("address").and_then(Value::as_str)?.trim();
    if chain.is_empty() || symbol.is_empty() || address.is_empty() {
        return None;
    }
    let decimals = obj.get("decimals").and_then(value_to_u64);
    let name = obj.get("name").and_then(Value::as_str).map(str::to_string);
    let tags = obj
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(TokenRegistryEntry {
        chain: chain.to_string(),
        symbol: symbol.to_string(),
        address: address.to_string(),
        decimals,
        name,
        tags,
    })
}

fn value_to_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn deep_merge_value(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_obj), Value::Object(patch_obj)) => {
            for (key, patch_value) in patch_obj {
                match base_obj.get_mut(key) {
                    Some(base_value) => deep_merge_value(base_value, patch_value),
                    None => {
                        base_obj.insert(key.clone(), patch_value.clone());
                    }
                }
            }
        }
        (base_value, patch_value) => {
            *base_value = patch_value.clone();
        }
    }
}

fn pack_identity(pack: &PackDocument) -> String {
    let name = pack
        .name
        .as_deref()
        .or_else(|| {
            pack.meta
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or("unknown-pack");
    let version = pack
        .version
        .as_deref()
        .or_else(|| {
            pack.meta
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("version"))
                .and_then(Value::as_str)
        })
        .unwrap_or("0.0.0");
    format!("{name}@{version}")
}

fn protocol_id(protocol: &ProtocolDocument) -> String {
    protocol
        .meta
        .as_object()
        .and_then(|meta| meta.get("protocol"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-protocol")
        .to_string()
}

fn operation_selector(protocol_id: &str, operation_key: &str) -> String {
    format!("{protocol_id}.{operation_key}")
}

fn operation_kind_label(kind: &ResolvedOperationKind) -> &'static str {
    match kind {
        ResolvedOperationKind::Action => "action",
        ResolvedOperationKind::Query => "query",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_deployment_for_chain, resolve_operation_spec, resolve_token_candidate_for_symbol,
        ResolvedOperationKind, TokenResolutionErrorCode,
    };
    use crate::documents::{PackDocument, ProtocolDocument};
    use serde_json::{json, Value};

    #[test]
    fn resolve_deployment_prefers_exact_chain_then_namespace_wildcard() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[
                {"chain":"eip155:*","contracts":{"router":"0xwild"}},
                {"chain":"eip155:1","contracts":{"router":"0xexact"}}
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");

        let resolved = resolve_deployment_for_chain(&protocol, "eip155:1").expect("deployment");
        assert_eq!(resolved.chain, "eip155:1");
        assert_eq!(resolved.contracts.get("router"), Some(&json!("0xexact")));
    }

    #[test]
    fn resolve_deployment_falls_back_to_namespace_wildcard() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[
                {"chain":"eip155:*","contracts":{"router":"0xwild"}},
                {"chain":"*","contracts":{"router":"0xany"}}
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");

        let resolved = resolve_deployment_for_chain(&protocol, "eip155:10").expect("deployment");
        assert_eq!(resolved.chain, "eip155:*");
        assert_eq!(resolved.contracts.get("router"), Some(&json!("0xwild")));
    }

    #[test]
    fn resolve_deployment_skips_non_object_contracts_shape() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[
                {"chain":"eip155:1","contracts":"0xbroken"}
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");

        assert!(resolve_deployment_for_chain(&protocol, "eip155:1").is_none());
    }

    #[test]
    fn resolve_operation_spec_merges_pack_action_override_and_constraints() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"uniswap-v3","version":"0.0.2"},
            "deployments":[{"chain":"eip155:8453","contracts":{"router":"0x1"}}],
            "actions":{
                "swap-exact-in":{
                    "description":"base",
                    "risk_level":3,
                    "requires_queries":["quote"],
                    "execution":{"eip155:*":{"type":"evm_call","to":{"lit":"0x1"},"abi":{"type":"function","name":"swap","inputs":[],"outputs":[]},"args":{}}}
                }
            },
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[{"protocol":"uniswap-v3","version":"0.0.2","source":"registry"}],
            "policy":{
                "constraints":[{"id":"global","effect":"hard_block","assert":"inputs.x"}]
            },
            "providers":{"quote":{"enabled":[{"provider":"quoter"}]}},
            "overrides":{
                "action_rules":[
                    {"id":"swap-rule","actions":["uniswap-v3.swap-exact-in"],"constraints":[{"id":"rule","effect":"hard_block","assert":"params.y"}]}
                ],
                "actions":{
                    "uniswap-v3.swap-exact-in":{
                        "description":"merged",
                        "requires_queries":["quote","allowance"],
                        "constraints":[{"id":"action","effect":"hard_block","assert":"params.z"}]
                    }
                }
            }
        }))
        .expect("pack");

        let resolved = resolve_operation_spec(
            "uniswap-v3@0.0.2",
            &protocol,
            "swap-exact-in",
            ResolvedOperationKind::Action,
            "eip155:8453",
            Some(&pack),
        )
        .expect("resolved");

        assert_eq!(
            resolved
                .merged_spec
                .get("description")
                .and_then(Value::as_str),
            Some("merged")
        );
        assert_eq!(
            resolved
                .merged_spec
                .get("requires_queries")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        let pack = resolved.pack.expect("pack");
        assert_eq!(pack.matched_action_rule_ids, vec!["swap-rule".to_string()]);
        assert_eq!(pack.effective_constraints.len(), 3);
        assert_eq!(
            pack.effective_constraints
                .iter()
                .filter_map(Value::as_object)
                .filter_map(|constraint| constraint.get("id"))
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["global", "rule", "action"]
        );
    }

    #[test]
    fn resolve_token_candidate_for_symbol_prefers_pack_allowlist() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[],
            "supported_assets":[
                {
                    "symbol":"USDC",
                    "addresses":{"eip155:1":"0xprotocol"},
                    "decimals":{"eip155:1":6}
                }
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[],
            "token_policy":{
                "resolution":{
                    "allow_symbol_input":true,
                    "require_user_confirm_asset_address":true,
                    "require_allowlist_for_symbol_resolution":true
                },
                "allowlist":[
                    {
                        "chain":"eip155:1",
                        "symbol":"USDC",
                        "address":"0xallowlist",
                        "decimals":6,
                        "tags":["stable"]
                    }
                ]
            }
        }))
        .expect("pack");

        let resolved =
            resolve_token_candidate_for_symbol(Some(&pack), Some(&protocol), "eip155:1", "USDC")
                .expect("resolved");
        assert_eq!(resolved.address, "0xallowlist");
        assert_eq!(resolved.source, "pack_allowlist");
        assert!(resolved.allowlisted);
        assert!(resolved.require_user_confirm_asset_address);
    }

    #[test]
    fn resolve_token_candidate_for_symbol_falls_back_to_protocol_supported_assets() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[],
            "supported_assets":[
                {
                    "symbol":"WETH",
                    "name":"Wrapped Ether",
                    "addresses":{"eip155:1":"0x4200000000000000000000000000000000000006"},
                    "decimals":{"eip155:1":18},
                    "tags":["wrapped"]
                }
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[],
            "token_policy":{
                "resolution":{
                    "allow_symbol_input":true
                }
            }
        }))
        .expect("pack");

        let resolved =
            resolve_token_candidate_for_symbol(Some(&pack), Some(&protocol), "eip155:1", "WETH")
                .expect("resolved");
        assert_eq!(
            resolved.address,
            "0x4200000000000000000000000000000000000006"
        );
        assert_eq!(resolved.source, "protocol_supported_assets");
        assert!(!resolved.allowlisted);
    }

    #[test]
    fn resolve_token_candidate_for_symbol_supports_deployment_list_asset_shape() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[],
            "supported_assets":[
                {
                    "symbol":"USDC",
                    "name":"USD Coin",
                    "deployments":[
                        {"chain":"eip155:8453","address":"0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913","decimals":6}
                    ],
                    "tags":["stable"]
                }
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[],
            "token_policy":{
                "resolution":{"allow_symbol_input":true}
            }
        }))
        .expect("pack");

        let resolved =
            resolve_token_candidate_for_symbol(Some(&pack), Some(&protocol), "eip155:8453", "USDC")
                .expect("resolved");
        assert_eq!(
            resolved.address,
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        );
        assert_eq!(resolved.decimals, Some(6));
        assert_eq!(resolved.source, "protocol_supported_assets");
    }

    #[test]
    fn resolve_token_candidate_for_symbol_rejects_non_allowlisted_symbol_when_required() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[],
            "supported_assets":[
                {
                    "symbol":"USDC",
                    "addresses":{"eip155:1":"0xprotocol"},
                    "decimals":{"eip155:1":6}
                }
            ],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[],
            "token_policy":{
                "resolution":{
                    "allow_symbol_input":true,
                    "require_allowlist_for_symbol_resolution":true
                },
                "allowlist":[]
            }
        }))
        .expect("pack");

        let error =
            resolve_token_candidate_for_symbol(Some(&pack), Some(&protocol), "eip155:1", "USDC")
                .expect_err("must reject");
        assert_eq!(error.code, TokenResolutionErrorCode::SymbolNotAllowlisted);
    }

    #[test]
    fn resolve_token_candidate_for_symbol_rejects_symbol_input_when_disabled() {
        let protocol: ProtocolDocument = serde_json::from_value(json!({
            "schema":"ais/0.0.2",
            "meta":{"protocol":"demo","version":"0.0.2"},
            "deployments":[],
            "supported_assets":[],
            "actions":{},
            "queries":{}
        }))
        .expect("protocol");
        let pack: PackDocument = serde_json::from_value(json!({
            "schema":"ais-pack/0.0.2",
            "name":"safe-defi",
            "version":"0.0.2",
            "includes":[],
            "token_policy":{
                "resolution":{
                    "allow_symbol_input":false
                }
            }
        }))
        .expect("pack");

        let error =
            resolve_token_candidate_for_symbol(Some(&pack), Some(&protocol), "eip155:1", "USDC")
                .expect_err("must reject");
        assert_eq!(error.code, TokenResolutionErrorCode::SymbolInputNotAllowed);
    }
}
