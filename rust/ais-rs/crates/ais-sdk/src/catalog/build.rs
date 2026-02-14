use crate::documents::{CatalogDocument, PackDocument, ProtocolDocument, WorkflowDocument};
use ais_core::{stable_hash_hex, StableJsonOptions};
use ais_schema::versions::SCHEMA_CATALOG_0_0_1;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub struct CatalogBuildInput<'a> {
    pub protocols: &'a [ProtocolDocument],
    pub packs: &'a [PackDocument],
    pub workflows: &'a [WorkflowDocument],
}

#[derive(Debug, Clone, Default)]
pub struct CatalogBuildOptions {
    pub created_at: Option<String>,
}

pub fn build_catalog(
    input: CatalogBuildInput<'_>,
    options: &CatalogBuildOptions,
) -> serde_json::Result<CatalogDocument> {
    let mut actions = Vec::new();
    let mut queries = Vec::new();
    let mut packs = Vec::new();
    let mut documents = Vec::new();

    for protocol in input.protocols {
        let protocol_id = protocol_meta_field(protocol, "protocol").unwrap_or("unknown-protocol");
        let version = protocol_meta_field(protocol, "version").unwrap_or("0.0.0");
        let doc_id = format!("{protocol_id}@{version}");
        documents.push(json!({
            "kind": "protocol",
            "id": doc_id,
            "hash": stable_hash_hex(&protocol_fingerprint(protocol), &StableJsonOptions::default())?,
        }));

        for action_id in protocol.actions.keys() {
            actions.push(action_card(protocol, action_id));
        }
        for query_id in protocol.queries.keys() {
            queries.push(query_card(protocol, query_id));
        }
    }

    for pack in input.packs {
        let (name, version) = pack_identity(pack);
        let doc_id = format!("{name}@{version}");
        documents.push(json!({
            "kind": "pack",
            "id": doc_id,
            "hash": stable_hash_hex(&pack_fingerprint(pack), &StableJsonOptions::default())?,
        }));
        packs.push(pack_card(pack));
    }

    for workflow in input.workflows {
        let name = workflow
            .meta
            .as_object()
            .and_then(|meta| meta.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown-workflow");
        let version = workflow
            .meta
            .as_object()
            .and_then(|meta| meta.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("0.0.0");
        documents.push(json!({
            "kind": "workflow",
            "id": format!("{name}@{version}"),
            "hash": stable_hash_hex(
                &json!({
                    "schema": workflow.schema,
                    "name": name,
                    "version": version
                }),
                &StableJsonOptions::default()
            )?,
        }));
    }

    actions.sort_by(action_sort_key);
    queries.sort_by(query_sort_key);
    packs.sort_by(pack_sort_key);
    documents.sort_by(document_sort_key);

    let mut catalog = CatalogDocument {
        schema: SCHEMA_CATALOG_0_0_1.to_string(),
        created_at: options.created_at.clone(),
        hash: None,
        documents,
        actions,
        queries,
        packs,
        extensions: Map::new(),
    };

    catalog.hash = Some(catalog_hash(&catalog)?);
    Ok(catalog)
}

fn catalog_hash(catalog: &CatalogDocument) -> serde_json::Result<String> {
    let mut value = serde_json::to_value(catalog)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("hash");
    }
    let mut ignore_object_keys = BTreeSet::new();
    ignore_object_keys.insert("created_at".to_string());
    ignore_object_keys.insert("hash".to_string());
    let options = StableJsonOptions { ignore_object_keys };
    stable_hash_hex(&value, &options)
}

fn action_card(protocol: &ProtocolDocument, action_id: &str) -> Value {
    let protocol_id = protocol_meta_field(protocol, "protocol").unwrap_or("unknown-protocol");
    let version = protocol_meta_field(protocol, "version").unwrap_or("0.0.0");
    let spec = protocol.actions.get(action_id).cloned().unwrap_or(Value::Null);

    json!({
        "ref": format!("{protocol_id}@{version}/{action_id}"),
        "protocol": protocol_id,
        "version": version,
        "id": action_id,
        "execution_types": extract_execution_types(&spec),
        "execution_chains": extract_execution_chains(&spec),
        "capabilities_required": merge_capabilities(protocol, &spec),
    })
}

fn query_card(protocol: &ProtocolDocument, query_id: &str) -> Value {
    let protocol_id = protocol_meta_field(protocol, "protocol").unwrap_or("unknown-protocol");
    let version = protocol_meta_field(protocol, "version").unwrap_or("0.0.0");
    let spec = protocol.queries.get(query_id).cloned().unwrap_or(Value::Null);

    json!({
        "ref": format!("{protocol_id}@{version}/{query_id}"),
        "protocol": protocol_id,
        "version": version,
        "id": query_id,
        "execution_types": extract_execution_types(&spec),
        "execution_chains": extract_execution_chains(&spec),
        "capabilities_required": merge_capabilities(protocol, &spec),
    })
}

fn pack_card(pack: &PackDocument) -> Value {
    let (name, version) = pack_identity(pack);
    let mut includes = Vec::new();
    for include in &pack.includes {
        let Some(inc_obj) = include.as_object() else {
            continue;
        };
        let Some(protocol) = inc_obj.get("protocol").and_then(Value::as_str) else {
            continue;
        };
        let Some(inc_version) = inc_obj.get("version").and_then(Value::as_str) else {
            continue;
        };

        let mut include_card = json!({
            "protocol": protocol,
            "version": inc_version
        });

        if let Some(scope) = inc_obj.get("chain_scope").and_then(Value::as_array) {
            let mut chains: Vec<String> = scope
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            chains.sort();
            chains.dedup();
            if let Some(obj) = include_card.as_object_mut() {
                obj.insert(
                    "chain_scope".to_string(),
                    Value::Array(chains.into_iter().map(Value::String).collect()),
                );
            }
        }

        includes.push(include_card);
    }

    includes.sort_by(|left, right| {
        (value_str(left, "protocol"), value_str(left, "version")).cmp(&(
            value_str(right, "protocol"),
            value_str(right, "version"),
        ))
    });

    json!({
        "name": name,
        "version": version,
        "includes": includes,
        "description": pack.description
            .clone()
            .or_else(|| pack.meta.as_ref().and_then(Value::as_object).and_then(|meta| meta.get("description")).and_then(Value::as_str).map(str::to_string)),
    })
}

fn protocol_fingerprint(protocol: &ProtocolDocument) -> Value {
    let protocol_id = protocol_meta_field(protocol, "protocol").unwrap_or("unknown-protocol");
    let version = protocol_meta_field(protocol, "version").unwrap_or("0.0.0");
    let mut actions: Vec<String> = protocol.actions.keys().cloned().collect();
    let mut queries: Vec<String> = protocol.queries.keys().cloned().collect();
    actions.sort();
    queries.sort();
    let mut chains = protocol
        .deployments
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|deployment| deployment.get("chain"))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    chains.sort();
    chains.dedup();

    json!({
        "schema": protocol.schema,
        "protocol": protocol_id,
        "version": version,
        "actions": actions,
        "queries": queries,
        "deployments": chains,
    })
}

fn pack_fingerprint(pack: &PackDocument) -> Value {
    let (name, version) = pack_identity(pack);
    let mut includes = pack
        .includes
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|include| {
            Some(format!(
                "{}@{}",
                include.get("protocol")?.as_str()?,
                include.get("version")?.as_str()?
            ))
        })
        .collect::<Vec<_>>();
    includes.sort();
    includes.dedup();

    json!({
        "schema": pack.schema,
        "name": name,
        "version": version,
        "includes": includes,
    })
}

fn protocol_meta_field<'a>(protocol: &'a ProtocolDocument, key: &str) -> Option<&'a str> {
    protocol
        .meta
        .as_object()
        .and_then(|meta| meta.get(key))
        .and_then(Value::as_str)
}

fn pack_identity(pack: &PackDocument) -> (String, String) {
    let meta_name = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("name"))
        .and_then(Value::as_str);
    let meta_version = pack
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str);

    (
        meta_name
            .or(pack.name.as_deref())
            .unwrap_or("unknown-pack")
            .to_string(),
        meta_version
            .or(pack.version.as_deref())
            .unwrap_or("0.0.0")
            .to_string(),
    )
}

fn merge_capabilities(protocol: &ProtocolDocument, spec: &Value) -> Vec<String> {
    let mut values = Vec::<String>::new();
    values.extend(protocol.capabilities_required.clone());
    values.extend(
        spec.as_object()
            .and_then(|obj| obj.get("capabilities_required"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string),
    );
    values.retain(|value| !value.trim().is_empty());
    values.sort();
    values.dedup();
    values
}

fn extract_execution_chains(spec: &Value) -> Vec<String> {
    let mut chains = spec
        .as_object()
        .and_then(|obj| obj.get("execution"))
        .and_then(Value::as_object)
        .map(|execution| execution.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    chains.sort();
    chains
}

fn extract_execution_types(spec: &Value) -> Vec<String> {
    let mut types = Vec::<String>::new();
    if let Some(execution) = spec
        .as_object()
        .and_then(|obj| obj.get("execution"))
        .and_then(Value::as_object)
    {
        for entry in execution.values() {
            collect_execution_type(entry, &mut types);
        }
    }
    types.sort();
    types.dedup();
    types
}

fn collect_execution_type(spec: &Value, out: &mut Vec<String>) {
    let Some(spec_obj) = spec.as_object() else {
        return;
    };
    let Some(exec_type) = spec_obj.get("type").and_then(Value::as_str) else {
        return;
    };
    out.push(exec_type.to_string());

    if exec_type == "composite" {
        if let Some(steps) = spec_obj.get("steps").and_then(Value::as_array) {
            for step in steps {
                if let Some(child_exec) = step
                    .as_object()
                    .and_then(|step_obj| step_obj.get("execution"))
                {
                    collect_execution_type(child_exec, out);
                }
            }
        }
    }
}

fn action_sort_key(left: &Value, right: &Value) -> std::cmp::Ordering {
    (
        value_str(left, "protocol"),
        value_str(left, "version"),
        value_str(left, "id"),
    )
        .cmp(&(
            value_str(right, "protocol"),
            value_str(right, "version"),
            value_str(right, "id"),
        ))
}

fn query_sort_key(left: &Value, right: &Value) -> std::cmp::Ordering {
    (
        value_str(left, "protocol"),
        value_str(left, "version"),
        value_str(left, "id"),
    )
        .cmp(&(
            value_str(right, "protocol"),
            value_str(right, "version"),
            value_str(right, "id"),
        ))
}

fn pack_sort_key(left: &Value, right: &Value) -> std::cmp::Ordering {
    (value_str(left, "name"), value_str(left, "version")).cmp(&(
        value_str(right, "name"),
        value_str(right, "version"),
    ))
}

fn document_sort_key(left: &Value, right: &Value) -> std::cmp::Ordering {
    (value_str(left, "kind"), value_str(left, "id")).cmp(&(
        value_str(right, "kind"),
        value_str(right, "id"),
    ))
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[cfg(test)]
#[path = "build_test.rs"]
mod tests;
