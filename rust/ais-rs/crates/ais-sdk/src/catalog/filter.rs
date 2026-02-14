use crate::catalog::index::{build_index_from_parts, CatalogIndex};
use crate::documents::PackDocument;
use ais_core::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCapabilities {
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub execution_types: Vec<String>,
    #[serde(default)]
    pub detect_kinds: Vec<String>,
}

pub const EXECUTABLE_CANDIDATES_SCHEMA_0_0_1: &str = "ais-executable-candidates/0.0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutableCandidates {
    pub schema: String,
    pub created_at: Option<String>,
    pub hash: String,
    pub catalog_schema: String,
    pub catalog_hash: String,
    #[serde(default)]
    pub pack: Option<Value>,
    #[serde(default)]
    pub chain_scope: Option<Vec<String>>,
    pub actions: Vec<Value>,
    pub queries: Vec<Value>,
    pub detect_providers: Vec<Value>,
    pub execution_plugins: Vec<Value>,
}

pub fn filter_by_pack(index: &CatalogIndex, pack: &PackDocument) -> CatalogIndex {
    let include_by_protocol_version = build_pack_include_map(pack);

    let mut actions = Vec::new();
    for action in &index.actions {
        let Some(protocol) = value_str_opt(action, "protocol") else {
            continue;
        };
        let Some(version) = value_str_opt(action, "version") else {
            continue;
        };
        let key = format!("{protocol}@{version}");
        let Some(scope) = include_by_protocol_version.get(&key) else {
            continue;
        };

        if let Some(trimmed) = trim_card_by_chain_scope(action, scope) {
            actions.push(trimmed);
        }
    }

    let mut queries = Vec::new();
    for query in &index.queries {
        let Some(protocol) = value_str_opt(query, "protocol") else {
            continue;
        };
        let Some(version) = value_str_opt(query, "version") else {
            continue;
        };
        let key = format!("{protocol}@{version}");
        let Some(scope) = include_by_protocol_version.get(&key) else {
            continue;
        };

        if let Some(trimmed) = trim_card_by_chain_scope(query, scope) {
            queries.push(trimmed);
        }
    }

    let mut out = build_index_from_parts(
        &index.catalog_schema,
        &index.catalog_hash,
        actions,
        queries,
        index.packs.clone(),
    );

    out.detect_providers = Some(derive_detect_providers(pack));
    out.execution_plugins = Some(derive_execution_plugins(pack));
    out
}

pub fn filter_by_engine_capabilities(index: &CatalogIndex, capabilities: &EngineCapabilities) -> CatalogIndex {
    let supported_caps: HashSet<String> = capabilities
        .capabilities
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();
    let supported_execution_types: HashSet<String> = capabilities
        .execution_types
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();
    let supported_detect_kinds: HashSet<String> = capabilities
        .detect_kinds
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();

    let actions = index
        .actions
        .iter()
        .filter(|action| card_matches_capabilities(action, &supported_caps, &supported_execution_types))
        .cloned()
        .collect::<Vec<_>>();

    let queries = index
        .queries
        .iter()
        .filter(|query| card_matches_capabilities(query, &supported_caps, &supported_execution_types))
        .cloned()
        .collect::<Vec<_>>();

    let mut out = build_index_from_parts(
        &index.catalog_schema,
        &index.catalog_hash,
        actions,
        queries,
        index.packs.clone(),
    );

    if let Some(providers) = &index.detect_providers {
        out.detect_providers = Some(
            providers
                .iter()
                .filter(|provider| {
                    if supported_detect_kinds.is_empty() {
                        return true;
                    }
                    value_str_opt(provider, "kind")
                        .map(|kind| supported_detect_kinds.contains(kind))
                        .unwrap_or(false)
                })
                .cloned()
                .collect(),
        );
    }

    if let Some(plugins) = &index.execution_plugins {
        out.execution_plugins = Some(
            plugins
                .iter()
                .filter(|plugin| {
                    if supported_execution_types.is_empty() {
                        return true;
                    }
                    value_str_opt(plugin, "type")
                        .map(|value| supported_execution_types.contains(value))
                        .unwrap_or(false)
                })
                .cloned()
                .collect(),
        );
    }

    out
}

pub fn get_executable_candidates(
    index: &CatalogIndex,
    pack: Option<&PackDocument>,
    engine_capabilities: Option<&EngineCapabilities>,
    chain_scope: Option<&[String]>,
    created_at: Option<String>,
) -> serde_json::Result<ExecutableCandidates> {
    let mut scoped_index = index.clone();
    if let Some(pack) = pack {
        scoped_index = filter_by_pack(&scoped_index, pack);
    }
    if let Some(engine_capabilities) = engine_capabilities {
        scoped_index = filter_by_engine_capabilities(&scoped_index, engine_capabilities);
    }
    if let Some(scope) = chain_scope {
        scoped_index = apply_chain_scope(&scoped_index, scope);
    }

    let mut actions = scoped_index
        .actions
        .iter()
        .cloned()
        .map(|mut action| {
            let id = value_str_opt(&action, "id").map(str::to_string);
            if let Some(id) = id {
                if let Some(obj) = action.as_object_mut() {
                    obj.insert("signature".to_string(), Value::String(format!("{id}()")));
                }
            }
            action
        })
        .collect::<Vec<_>>();
    actions.sort_by(|left, right| value_str(left, "ref").cmp(value_str(right, "ref")));

    let mut queries = scoped_index
        .queries
        .iter()
        .cloned()
        .map(|mut query| {
            let id = value_str_opt(&query, "id").map(str::to_string);
            if let Some(id) = id {
                if let Some(obj) = query.as_object_mut() {
                    obj.insert("signature".to_string(), Value::String(format!("{id}()")));
                }
            }
            query
        })
        .collect::<Vec<_>>();
    queries.sort_by(|left, right| value_str(left, "ref").cmp(value_str(right, "ref")));

    let mut detect_providers = explode_detect_providers(
        scoped_index.detect_providers.as_deref().unwrap_or(&[]),
    );
    let mut execution_plugins = explode_execution_plugins(
        scoped_index.execution_plugins.as_deref().unwrap_or(&[]),
    );
    if let Some(scope) = chain_scope {
        let scope_set: HashSet<String> = scope.iter().cloned().collect();
        detect_providers.retain(|provider| {
            value_str_opt(provider, "chain")
                .map(|chain| scope_set.contains(chain))
                .unwrap_or(true)
        });
        execution_plugins.retain(|plugin| {
            value_str_opt(plugin, "chain")
                .map(|chain| scope_set.contains(chain))
                .unwrap_or(true)
        });
    }
    detect_providers.sort_by(|left, right| {
        (
            value_str(left, "kind"),
            value_str(left, "chain"),
            -value_i64(left, "priority"),
            value_str(left, "provider"),
        )
            .cmp(&(
                value_str(right, "kind"),
                value_str(right, "chain"),
                -value_i64(right, "priority"),
                value_str(right, "provider"),
            ))
    });
    execution_plugins.sort_by(|left, right| {
        (value_str(left, "type"), value_str(left, "chain")).cmp(&(
            value_str(right, "type"),
            value_str(right, "chain"),
        ))
    });

    let pack_value = pack.map(pack_identity_value);
    let chain_scope_value = chain_scope.map(|scope| {
        let mut out = scope.to_vec();
        out.sort();
        out.dedup();
        out
    });

    let hash = {
        let mut content = json!({
            "schema": EXECUTABLE_CANDIDATES_SCHEMA_0_0_1,
            "catalog_schema": scoped_index.catalog_schema,
            "catalog_hash": scoped_index.catalog_hash,
            "pack": pack_value,
            "chain_scope": chain_scope_value,
            "actions": actions,
            "queries": queries,
            "detect_providers": detect_providers,
            "execution_plugins": execution_plugins,
        });
        if let Some(obj) = content.as_object_mut() {
            obj.remove("created_at");
            obj.remove("hash");
        }
        let mut ignore_object_keys = BTreeSet::new();
        ignore_object_keys.insert("created_at".to_string());
        ignore_object_keys.insert("hash".to_string());
        stable_hash_hex(&content, &StableJsonOptions { ignore_object_keys })?
    };

    Ok(ExecutableCandidates {
        schema: EXECUTABLE_CANDIDATES_SCHEMA_0_0_1.to_string(),
        created_at,
        hash,
        catalog_schema: scoped_index.catalog_schema.clone(),
        catalog_hash: scoped_index.catalog_hash.clone(),
        pack: pack_value,
        chain_scope: chain_scope_value,
        actions,
        queries,
        detect_providers,
        execution_plugins,
    })
}

fn build_pack_include_map(pack: &PackDocument) -> HashMap<String, Option<HashSet<String>>> {
    let mut out = HashMap::new();
    for include in &pack.includes {
        let Some(include_obj) = include.as_object() else {
            continue;
        };
        let Some(protocol) = include_obj.get("protocol").and_then(Value::as_str) else {
            continue;
        };
        let Some(version) = include_obj.get("version").and_then(Value::as_str) else {
            continue;
        };
        let key = format!("{protocol}@{version}");
        let chain_scope = include_obj
            .get("chain_scope")
            .and_then(Value::as_array)
            .map(|chains| {
                chains
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<HashSet<_>>()
            });
        out.insert(key, chain_scope);
    }
    out
}

fn trim_card_by_chain_scope(card: &Value, chain_scope: &Option<HashSet<String>>) -> Option<Value> {
    match chain_scope {
        None => Some(card.clone()),
        Some(scope) if scope.is_empty() => Some(card.clone()),
        Some(scope) => {
            let card_obj = card.as_object()?;
            let chains = card_obj
                .get("execution_chains")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|chain| scope.contains(*chain))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if chains.is_empty() {
                return None;
            }

            let mut next = card.clone();
            if let Some(next_obj) = next.as_object_mut() {
                next_obj.insert(
                    "execution_chains".to_string(),
                    Value::Array(chains.into_iter().map(Value::String).collect()),
                );
            }
            Some(next)
        }
    }
}

fn card_matches_capabilities(
    card: &Value,
    supported_caps: &HashSet<String>,
    supported_execution_types: &HashSet<String>,
) -> bool {
    if !supported_caps.is_empty() {
        let required_caps = card
            .as_object()
            .and_then(|obj| obj.get("capabilities_required"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        if required_caps.iter().any(|cap| !supported_caps.contains(*cap)) {
            return false;
        }
    }

    if !supported_execution_types.is_empty() {
        let execution_types = card
            .as_object()
            .and_then(|obj| obj.get("execution_types"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        if execution_types
            .iter()
            .any(|execution_type| !supported_execution_types.contains(*execution_type))
        {
            return false;
        }
    }

    true
}

fn derive_detect_providers(pack: &PackDocument) -> Vec<Value> {
    let providers = pack
        .providers
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|providers| providers.get("detect"))
        .and_then(Value::as_object)
        .and_then(|detect| detect.get("enabled"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = providers
        .into_iter()
        .filter_map(|entry| {
            let entry_obj = entry.as_object()?;
            let kind = entry_obj.get("kind")?.as_str()?;
            let provider = entry_obj.get("provider")?.as_str()?;
            let priority = entry_obj.get("priority").and_then(Value::as_i64).unwrap_or(0);
            let mut chains = entry_obj
                .get("chains")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            chains.sort();
            chains.dedup();
            Some(json!({
                "kind": kind,
                "provider": provider,
                "chains": chains,
                "priority": priority
            }))
        })
        .collect::<Vec<_>>();

    out.sort_by(|left, right| {
        (
            value_str(left, "kind"),
            -value_i64(left, "priority"),
            value_str(left, "provider"),
        )
            .cmp(&(
                value_str(right, "kind"),
                -value_i64(right, "priority"),
                value_str(right, "provider"),
            ))
    });
    out
}

fn derive_execution_plugins(pack: &PackDocument) -> Vec<Value> {
    let plugins = pack
        .plugins
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("execution"))
        .and_then(Value::as_object)
        .and_then(|execution| execution.get("enabled"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = plugins
        .into_iter()
        .filter_map(|entry| {
            let entry_obj = entry.as_object()?;
            let typ = entry_obj.get("type")?.as_str()?;
            let mut chains = entry_obj
                .get("chains")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            chains.sort();
            chains.dedup();
            Some(json!({
                "type": typ,
                "chains": chains
            }))
        })
        .collect::<Vec<_>>();

    out.sort_by(|left, right| value_str(left, "type").cmp(value_str(right, "type")));
    out
}

fn apply_chain_scope(index: &CatalogIndex, chain_scope: &[String]) -> CatalogIndex {
    let scope_set: HashSet<String> = chain_scope.iter().cloned().collect();
    let actions = index
        .actions
        .iter()
        .filter_map(|action| trim_card_by_chain_scope(action, &Some(scope_set.clone())))
        .collect::<Vec<_>>();
    let queries = index
        .queries
        .iter()
        .filter_map(|query| trim_card_by_chain_scope(query, &Some(scope_set.clone())))
        .collect::<Vec<_>>();

    let mut out = build_index_from_parts(
        &index.catalog_schema,
        &index.catalog_hash,
        actions,
        queries,
        index.packs.clone(),
    );
    out.detect_providers = index.detect_providers.clone();
    out.execution_plugins = index.execution_plugins.clone();
    out
}

fn explode_detect_providers(providers: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for provider in providers {
        let Some(provider_obj) = provider.as_object() else {
            continue;
        };
        let kind = provider_obj.get("kind").and_then(Value::as_str).unwrap_or("");
        let provider_name = provider_obj
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("");
        let priority = provider_obj.get("priority").and_then(Value::as_i64).unwrap_or(0);
        let chains = provider_obj.get("chains").and_then(Value::as_array);
        if let Some(chains) = chains {
            if chains.is_empty() {
                out.push(json!({
                    "kind": kind,
                    "provider": provider_name,
                    "priority": priority
                }));
            } else {
                for chain in chains.iter().filter_map(Value::as_str) {
                    out.push(json!({
                        "kind": kind,
                        "provider": provider_name,
                        "chain": chain,
                        "priority": priority
                    }));
                }
            }
        } else {
            out.push(json!({
                "kind": kind,
                "provider": provider_name,
                "priority": priority
            }));
        }
    }
    out
}

fn explode_execution_plugins(plugins: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for plugin in plugins {
        let Some(plugin_obj) = plugin.as_object() else {
            continue;
        };
        let typ = plugin_obj.get("type").and_then(Value::as_str).unwrap_or("");
        let chains = plugin_obj.get("chains").and_then(Value::as_array);
        if let Some(chains) = chains {
            if chains.is_empty() {
                out.push(json!({ "type": typ }));
            } else {
                for chain in chains.iter().filter_map(Value::as_str) {
                    out.push(json!({ "type": typ, "chain": chain }));
                }
            }
        } else {
            out.push(json!({ "type": typ }));
        }
    }
    out
}

fn pack_identity_value(pack: &PackDocument) -> Value {
    let name = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("name"))
        .and_then(Value::as_str)
        .or(pack.name.as_deref())
        .unwrap_or("unknown-pack");
    let version = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str)
        .or(pack.version.as_deref())
        .unwrap_or("0.0.0");
    json!({
        "name": name,
        "version": version
    })
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|obj| obj.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn value_str_opt<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.as_object()?.get(key)?.as_str()
}

fn value_i64(value: &Value, key: &str) -> i64 {
    value
        .as_object()
        .and_then(|obj| obj.get(key))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "filter_test.rs"]
mod tests;
