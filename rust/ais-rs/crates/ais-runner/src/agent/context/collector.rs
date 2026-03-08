use super::super::input_store::InputStore;
use ais_engine::EngineRunnerState;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const PRIORITY_SLOTS: &[&str] = &[
    "owner",
    "wallet.default",
    "chain",
    "chain_id",
    "chain_ref",
    "network",
    "recipient",
    "to",
    "from",
    "sender",
    "token",
    "token.address",
    "asset",
    "asset.address",
    "amount",
    "amount.human",
    "amount.atomic",
];

#[derive(Debug, Clone)]
pub(super) struct InputSlotProjection {
    #[allow(dead_code)]
    pub(super) value: Value,
    pub(super) resolved: BTreeMap<String, Value>,
    pub(super) missing: Vec<String>,
}

pub(super) fn build_input_slots_projection(
    state: &EngineRunnerState,
    input_store: Option<&InputStore>,
) -> InputSlotProjection {
    let mut resolved = BTreeMap::<String, Value>::new();

    if let Some(store) = input_store {
        collect_input_store_input_slots(store, &mut resolved);
    }

    let mut missing = BTreeSet::<String>::new();
    if let Some(required_facts) = state
        .runtime
        .pointer("/agent/todo_progress/current_todo/required_facts")
        .and_then(Value::as_array)
    {
        for required in required_facts.iter().filter_map(Value::as_str) {
            if let Some(slot) = to_slot_id(required) {
                if !resolved.contains_key(slot.as_str()) {
                    missing.insert(slot);
                }
            }
        }
    }
    if let Some(questions) = state
        .runtime
        .pointer("/agent/missing_required_input/questions")
        .and_then(Value::as_array)
    {
        for question in questions {
            let Some(slot) = question
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
            else {
                continue;
            };
            if !resolved.contains_key(slot.as_str()) {
                missing.insert(slot);
            }
        }
    }

    let mut resolved_items = Vec::<Value>::new();
    let mut canonical_refs = serde_json::Map::<String, Value>::new();
    for (slot, value) in &resolved {
        let canonical_ref = format!("inputs.{slot}");
        resolved_items.push(json!({
            "id": slot,
            "ref": canonical_ref,
            "value": value,
        }));
        canonical_refs.insert(slot.clone(), Value::String(canonical_ref));
    }
    let missing_items = missing
        .into_iter()
        .map(|slot| {
            json!({
                "id": slot,
                "ref": format!("inputs.{slot}"),
            })
        })
        .collect::<Vec<_>>();
    let missing_slots = missing_items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    let missing_count = missing_items.len();

    let value = json!({
        "resolved": resolved_items,
        "missing": missing_items,
        "canonical_refs": canonical_refs,
        "counts": {
            "resolved": resolved.len(),
            "missing": missing_count,
        }
    });
    InputSlotProjection {
        value,
        resolved,
        missing: missing_slots,
    }
}

#[allow(dead_code)]
pub(super) fn build_canonical_context_projection(resolved: &BTreeMap<String, Value>) -> Value {
    let mut chain_refs = Vec::<Value>::new();
    let mut account_refs = Vec::<Value>::new();
    let mut asset_refs = Vec::<Value>::new();
    let mut amount_refs = Vec::<Value>::new();

    let mut chain_seen = BTreeSet::<String>::new();
    let mut account_seen = BTreeSet::<String>::new();
    let mut asset_seen = BTreeSet::<String>::new();
    let mut amount_seen = BTreeSet::<String>::new();

    for (slot, value) in resolved {
        let canonical_ref = format!("inputs.{slot}");
        if let Some(chain_ref) = extract_chain_ref(slot, value) {
            let dedupe = format!("{slot}:{chain_ref}");
            if chain_seen.insert(dedupe) {
                chain_refs.push(json!({
                    "id": slot,
                    "ref": canonical_ref,
                    "chain_ref": chain_ref,
                }));
            }
        }

        if let Some(account_ref) = extract_account_ref(slot, value) {
            let dedupe = format!("{slot}:{account_ref}");
            if account_seen.insert(dedupe) {
                account_refs.push(json!({
                    "id": slot,
                    "ref": canonical_ref,
                    "role": slot_leaf(slot),
                    "account_ref": account_ref,
                }));
            }
        }

        if let Some(asset_ref) = extract_asset_ref(value) {
            let dedupe = format!(
                "{slot}:{}:{}",
                asset_ref
                    .get("address")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                asset_ref
                    .get("chain_ref")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            if asset_seen.insert(dedupe) {
                let mut item = serde_json::Map::<String, Value>::new();
                item.insert("id".to_string(), Value::String(slot.to_string()));
                item.insert("ref".to_string(), Value::String(canonical_ref.clone()));
                for (key, value) in asset_ref {
                    item.insert(key, value);
                }
                asset_refs.push(Value::Object(item));
            }
        }

        if let Some(amount_ref) = extract_amount_ref(slot, value) {
            let encoded = serde_json::to_string(&amount_ref).unwrap_or_else(|_| "{}".to_string());
            let dedupe = format!("{slot}:{encoded}");
            if amount_seen.insert(dedupe) {
                let mut item = serde_json::Map::<String, Value>::new();
                item.insert("id".to_string(), Value::String(slot.to_string()));
                item.insert("ref".to_string(), Value::String(canonical_ref));
                for (key, value) in amount_ref {
                    item.insert(key, value);
                }
                amount_refs.push(Value::Object(item));
            }
        }
    }

    json!({
        "chain_refs": chain_refs,
        "account_refs": account_refs,
        "asset_refs": asset_refs,
        "amount_refs": amount_refs,
        "counts": {
            "chain_refs": chain_seen.len(),
            "account_refs": account_seen.len(),
            "asset_refs": asset_seen.len(),
            "amount_refs": amount_seen.len(),
        }
    })
}

pub(super) fn build_input_registry_projection(
    resolved: &BTreeMap<String, Value>,
    missing: &[String],
) -> Value {
    let mut known_refs = BTreeSet::<String>::new();
    let mut entries = Vec::<Value>::new();
    for (slot, value) in resolved {
        let reference = format!("inputs.{slot}");
        known_refs.insert(reference.clone());
        entries.push(json!({
            "id": slot,
            "ref": reference,
            "status": "resolved",
            "type_hint": value_type_hint(value),
            "example": value,
        }));
    }
    for slot in missing {
        let reference = format!("inputs.{slot}");
        entries.push(json!({
            "id": slot,
            "ref": reference,
            "status": "missing",
            "required": true,
        }));
    }
    entries.sort_by(|left, right| {
        let left_ref = left.get("ref").and_then(Value::as_str).unwrap_or_default();
        let right_ref = right.get("ref").and_then(Value::as_str).unwrap_or_default();
        left_ref.cmp(right_ref)
    });
    json!({
        "schema": "ais-agent-input-registry/0.0.1",
        "entries": entries,
        "known_refs": known_refs.iter().cloned().collect::<Vec<_>>(),
        "counts": {
            "known_refs": known_refs.len(),
            "resolved": resolved.len(),
            "missing": missing.len(),
        }
    })
}

pub(super) fn build_node_output_refs_projection(state: &EngineRunnerState) -> Value {
    let Some(nodes) = state.runtime.pointer("/nodes").and_then(Value::as_object) else {
        return json!({
            "schema": "ais-agent-node-output-refs/0.0.1",
            "entries": [],
            "known_refs": [],
            "counts": {"steps": 0, "entries": 0, "known_refs": 0},
        });
    };

    let mut refs_by_step = BTreeMap::<String, BTreeSet<String>>::new();
    for (node_id, node) in nodes {
        let Some(outputs) = node.get("outputs") else {
            continue;
        };
        if outputs.is_null() {
            continue;
        }
        let step_id = node_step_id_hint(node_id.as_str());
        let output_fields = collect_output_field_paths(outputs, 24);
        for value_ref in output_fields
            .iter()
            .map(|path| format!("nodes.{step_id}.outputs.{path}"))
        {
            refs_by_step
                .entry(step_id.clone())
                .or_default()
                .insert(value_ref);
        }
    }

    let mut entries = refs_by_step
        .iter()
        .map(|(step_id, refs)| {
            json!({
                "step_id": step_id,
                "refs": refs.iter().cloned().collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| slot_order_key(left).cmp(&slot_order_key(right)));
    let known_refs = refs_by_step
        .values()
        .flat_map(|refs| refs.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    json!({
        "schema": "ais-agent-node-output-refs/0.0.1",
        "entries": entries.clone(),
        "known_refs": known_refs.clone(),
        "counts": {
            "steps": refs_by_step.len(),
            "entries": entries.len(),
            "known_refs": known_refs.len(),
        },
    })
}

#[allow(dead_code)]
pub(super) fn prioritize_keys(mut keys: Vec<String>, max_entries: usize) -> Vec<String> {
    keys.sort_by(|left, right| {
        slot_sort_key(left.as_str())
            .cmp(&slot_sort_key(right.as_str()))
            .then_with(|| left.cmp(right))
    });
    keys.dedup();
    keys.truncate(max_entries.max(1));
    keys
}

#[allow(dead_code)]
pub(super) fn registry_order_key(entry: &Value) -> (u8, u8, String) {
    let status_rank = match entry.get("status").and_then(Value::as_str) {
        Some("missing") => 0,
        Some("resolved") => 1,
        _ => 2,
    };
    let slot = entry
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| entry.get("ref").and_then(Value::as_str))
        .unwrap_or_default();
    (
        status_rank,
        slot_sort_key(slot),
        entry
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

pub(super) fn slot_order_key(value: &Value) -> (u8, String) {
    if let Some(slot) = value
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| value.get("ref").and_then(Value::as_str))
    {
        return (slot_sort_key(slot), slot.to_string());
    }
    if let Some(text) = value.as_str() {
        return (slot_sort_key(text), text.to_string());
    }
    let encoded = serde_json::to_string(value).unwrap_or_default();
    (slot_sort_key(encoded.as_str()), encoded)
}

pub(super) fn slot_sort_key(slot: &str) -> u8 {
    let lowered = slot.to_ascii_lowercase();
    if PRIORITY_SLOTS
        .iter()
        .any(|priority| lowered == *priority || lowered.ends_with(priority))
    {
        return 0;
    }
    if lowered.contains("owner")
        || lowered.contains("wallet")
        || lowered.contains("recipient")
        || lowered.contains("token")
        || lowered.contains("asset")
        || lowered.contains("amount")
        || lowered.contains("chain")
    {
        return 1;
    }
    2
}

fn node_step_id_hint(node_id: &str) -> String {
    let trimmed = node_id.trim();
    if trimmed.is_empty() {
        return "step".to_string();
    }
    if let Some((_, suffix)) = trimmed.rsplit_once("__") {
        let suffix = suffix.trim();
        if !suffix.is_empty() {
            return suffix.to_string();
        }
    }
    if let Some((_, suffix)) = trimmed.rsplit_once('/') {
        let suffix = suffix.trim();
        if !suffix.is_empty() {
            return suffix.to_string();
        }
    }
    trimmed.to_string()
}

fn collect_output_field_paths(value: &Value, max_fields: usize) -> Vec<String> {
    let mut out = Vec::<String>::new();
    collect_output_field_paths_inner(value, "", 0, max_fields.max(1), &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_output_field_paths_inner(
    value: &Value,
    prefix: &str,
    depth: usize,
    max_fields: usize,
    out: &mut Vec<String>,
) {
    if out.len() >= max_fields {
        return;
    }
    if depth > 3 {
        if !prefix.is_empty() {
            out.push(prefix.to_string());
        }
        return;
    }
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                if !prefix.is_empty() && runtime_value_readable(value) {
                    out.push(prefix.to_string());
                }
                return;
            }
            for (key, nested) in map {
                if out.len() >= max_fields {
                    break;
                }
                let key = key.trim();
                if key.is_empty() {
                    continue;
                }
                let path = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                match nested {
                    Value::Object(_) => {
                        collect_output_field_paths_inner(
                            nested,
                            path.as_str(),
                            depth + 1,
                            max_fields,
                            out,
                        );
                    }
                    Value::Array(_) => {
                        if runtime_value_readable(nested) {
                            out.push(path);
                        }
                    }
                    _ => {
                        if runtime_value_readable(nested) {
                            out.push(path);
                        }
                    }
                }
            }
        }
        Value::Array(_) => {
            if !prefix.is_empty() && runtime_value_readable(value) {
                out.push(prefix.to_string());
            }
        }
        _ => {
            if !prefix.is_empty() && runtime_value_readable(value) {
                out.push(prefix.to_string());
            }
        }
    }
}

fn runtime_value_readable(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

fn value_type_hint(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn collect_input_store_input_slots(store: &InputStore, out: &mut BTreeMap<String, Value>) {
    for slot in store.list_projected_ref_strings() {
        let Some(value) = store.get_projected(slot.as_str()).map(|entry| entry.value) else {
            continue;
        };
        out.entry(slot).or_insert_with(|| value);
    }
}

#[allow(dead_code)]
fn extract_chain_ref(slot: &str, value: &Value) -> Option<String> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    if !matches!(
        leaf.as_str(),
        "chain" | "chain_id" | "chain_ref" | "network"
    ) {
        return None;
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[allow(dead_code)]
fn extract_account_ref(slot: &str, value: &Value) -> Option<String> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    let hint = [
        "owner",
        "from",
        "sender",
        "recipient",
        "to",
        "wallet",
        "account",
        "payer",
        "authority",
    ]
    .iter()
    .any(|name| leaf == *name || leaf.ends_with(name));
    if !hint {
        return None;
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[allow(dead_code)]
fn extract_asset_ref(value: &Value) -> Option<serde_json::Map<String, Value>> {
    let object = value.as_object()?;
    let address = object
        .get("address")
        .or_else(|| object.get("mint"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut out = serde_json::Map::<String, Value>::new();
    out.insert("address".to_string(), Value::String(address.to_string()));
    if let Some(chain_ref) = object
        .get("chain_ref")
        .or_else(|| object.get("chain_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.insert(
            "chain_ref".to_string(),
            Value::String(chain_ref.to_string()),
        );
    }
    if let Some(symbol) = object
        .get("symbol")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        out.insert("symbol".to_string(), Value::String(symbol.to_string()));
    }
    if let Some(decimals) = object.get("decimals").and_then(Value::as_u64) {
        out.insert("decimals".to_string(), Value::Number(decimals.into()));
    }
    Some(out)
}

#[allow(dead_code)]
fn extract_amount_ref(slot: &str, value: &Value) -> Option<serde_json::Map<String, Value>> {
    let leaf = slot_leaf(slot).to_ascii_lowercase();
    let is_amount_slot =
        leaf.contains("amount") || matches!(leaf.as_str(), "value" | "qty" | "quantity" | "size");
    if !is_amount_slot {
        return None;
    }
    let mut out = serde_json::Map::<String, Value>::new();
    match value {
        Value::Null | Value::Bool(_) => None,
        Value::Object(object) => {
            if let Some(human) = object.get("human") {
                out.insert("amount_human".to_string(), human.clone());
            }
            if let Some(atomic) = object.get("atomic") {
                out.insert("amount_atomic".to_string(), atomic.clone());
            }
            if out.is_empty() {
                out.insert("amount_ref".to_string(), value.clone());
            }
            Some(out)
        }
        _ => {
            out.insert("amount_ref".to_string(), value.clone());
            Some(out)
        }
    }
}

#[allow(dead_code)]
fn slot_leaf(slot: &str) -> &str {
    slot.rsplit('.').next().unwrap_or(slot)
}

fn to_slot_id(required_fact: &str) -> Option<String> {
    let trimmed = required_fact.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(slot) = trimmed.strip_prefix("inputs.") {
        let slot = slot.trim();
        if slot.is_empty() {
            return None;
        }
        return Some(slot.to_string());
    }
    if trimmed.starts_with("facts.")
        || trimmed.starts_with("tx.")
        || trimmed.starts_with("nodes.")
        || trimmed.starts_with("query.")
    {
        return None;
    }
    Some(trimmed.to_string())
}
