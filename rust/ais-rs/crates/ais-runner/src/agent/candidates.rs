use crate::cli::AgentCommand;
use crate::error::RunnerError;
use crate::io::load_workspace_documents;
use ais_sdk::{
    build_catalog, build_catalog_index, get_executable_candidates, CatalogBuildInput,
    CatalogBuildOptions, CatalogCardLevel, ExecutableCandidates, PackDocument, ProtocolDocument,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_MAX_INDEX_CANDIDATES: usize = 24;
pub const DEFAULT_MAX_DETAIL_REFS: usize = 16;
pub const DEFAULT_SEARCH_LIMIT: usize = 12;
pub const MAX_SEARCH_LIMIT: usize = 24;

#[derive(Debug, Clone, Default)]
pub struct CandidateSearchRequest {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub chain: Option<String>,
    pub min_risk_level: Option<u8>,
    pub max_risk_level: Option<u8>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CandidateContext {
    pub index_candidates: Value,
    pub detail_by_ref: BTreeMap<String, Value>,
    pub executable_candidates: ExecutableCandidates,
    pub protocols: Vec<ProtocolDocument>,
}

impl Default for CandidateContext {
    fn default() -> Self {
        Self {
            index_candidates: json!({
                "schema":"ais-executable-candidates/0.0.1",
                "actions":[],
                "queries":[]
            }),
            detail_by_ref: BTreeMap::new(),
            executable_candidates: ExecutableCandidates {
                schema: "ais-executable-candidates/0.0.1".to_string(),
                created_at: None,
                hash: String::new(),
                catalog_schema: "ais-catalog/0.0.1".to_string(),
                catalog_hash: String::new(),
                pack: None,
                chain_scope: None,
                actions: vec![],
                queries: vec![],
                execution_plugins: vec![],
            },
            protocols: vec![],
        }
    }
}

impl CandidateContext {
    pub fn get_details_for_refs(&self, refs: &[String]) -> Value {
        let requested = refs.len();
        let refs = refs
            .iter()
            .take(DEFAULT_MAX_DETAIL_REFS)
            .cloned()
            .collect::<Vec<_>>();
        let details = refs
            .iter()
            .filter_map(|reference| self.detail_by_ref.get(reference))
            .cloned()
            .collect::<Vec<_>>();
        json!({
            "schema": "ais-catalog-detail-response/0.0.1",
            "requested_refs": requested,
            "returned_refs": details.len(),
            "truncated": requested > DEFAULT_MAX_DETAIL_REFS,
            "count": details.len(),
            "details": details,
        })
    }

    pub fn search_candidates(&self, request: &CandidateSearchRequest) -> Value {
        let query_tokens = request
            .query
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| normalize_search_query_tokens(value.as_str()))
            .filter(|tokens| !tokens.is_empty());
        let kind = request
            .kind
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_lowercase())
            .unwrap_or_else(|| "any".to_string());
        let chain = request
            .chain
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        let limit = request
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        let mut matches = Vec::<Value>::new();

        if kind == "any" || kind == "action" {
            for action in &self.executable_candidates.actions {
                if !matches_keyword(action, query_tokens.as_deref()) {
                    continue;
                }
                if !matches_chain(action, chain) {
                    continue;
                }
                if !matches_risk(action, request.min_risk_level, request.max_risk_level, true) {
                    continue;
                }
                matches.push(to_discovery_card(action, "action"));
            }
        }

        if kind == "any" || kind == "query" {
            for query_card in &self.executable_candidates.queries {
                if !matches_keyword(query_card, query_tokens.as_deref()) {
                    continue;
                }
                if !matches_chain(query_card, chain) {
                    continue;
                }
                if !matches_risk(
                    query_card,
                    request.min_risk_level,
                    request.max_risk_level,
                    false,
                ) {
                    continue;
                }
                matches.push(to_discovery_card(query_card, "query"));
            }
        }

        let total_matches = matches.len();
        matches.truncate(limit);
        let returned_matches = matches.len();
        let truncated = returned_matches < total_matches;

        json!({
            "schema": "ais-catalog-search-response/0.0.1",
            "level": "name_only",
            "query": request.query.clone(),
            "filters": {
                "kind": kind,
                "chain": request.chain.clone(),
                "min_risk_level": request.min_risk_level,
                "max_risk_level": request.max_risk_level
            },
            "limit": limit,
            "total_matches": total_matches,
            "returned_matches": returned_matches,
            "truncated": truncated,
            "results": matches,
        })
    }

    pub fn capability_view(&self) -> Value {
        let mut per_protocol = BTreeMap::<String, Value>::new();
        let mut global_topics = BTreeSet::<String>::new();
        let semantic_hints_by_ref = build_semantic_hints_by_ref(&self.protocols);
        for card in self
            .executable_candidates
            .actions
            .iter()
            .chain(self.executable_candidates.queries.iter())
        {
            let Some(reference) = card.get("ref").and_then(Value::as_str) else {
                continue;
            };
            let Some((protocol, name)) = reference.split_once('/') else {
                continue;
            };
            let entry = per_protocol.entry(protocol.to_string()).or_insert_with(|| {
                json!({
                    "protocol": protocol,
                    "chains": Vec::<String>::new(),
                    "actions": Vec::<Value>::new(),
                    "queries": Vec::<Value>::new(),
                    "required_inputs": Vec::<String>::new(),
                    "topics": Vec::<String>::new(),
                    "topic_cards": Vec::<Value>::new(),
                })
            });
            if !entry.is_object() {
                *entry = json!({
                    "protocol": protocol,
                    "chains": Vec::<String>::new(),
                    "actions": Vec::<Value>::new(),
                    "queries": Vec::<Value>::new(),
                    "required_inputs": Vec::<String>::new(),
                    "topics": Vec::<String>::new(),
                    "topic_cards": Vec::<Value>::new(),
                });
            }
            let detail = self.detail_by_ref.get(reference);
            if let Some(chains) = card.get("execution_chains").and_then(Value::as_array) {
                merge_unique_strings(entry, "chains", chains);
            }
            let is_action = self
                .executable_candidates
                .actions
                .iter()
                .any(|action| action.get("ref").and_then(Value::as_str) == Some(reference));
            let required_inputs = self.required_inputs_for_ref(reference);
            let kind = if is_action { "action" } else { "query" };
            let fallback_risk_tags = detail
                .and_then(|value| value.get("risk_tags"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let hints = semantic_hints_by_ref.get(reference);
            let declared_topics = hints
                .map(|hint| hint.declared_topics.as_slice())
                .unwrap_or(&[]);
            let risk_tags = hints
                .map(|hint| hint.risk_tags.as_slice())
                .filter(|items| !items.is_empty())
                .unwrap_or(fallback_risk_tags.as_slice());
            let protocol_tags = hints
                .map(|hint| hint.protocol_tags.as_slice())
                .unwrap_or(&[]);
            let topic =
                semantic_topic_for_candidate(kind, declared_topics, risk_tags, protocol_tags);
            global_topics.insert(topic.clone());
            if is_action {
                push_unique_named_item(entry, "actions", name, reference, &required_inputs, &topic);
            } else {
                push_unique_named_item(entry, "queries", name, reference, &required_inputs, &topic);
            }
            merge_unique_required_inputs(entry, &required_inputs);
            merge_unique_protocol_topic(entry, &topic);
            merge_topic_card(
                entry,
                &topic,
                if is_action { "action" } else { "query" },
                reference,
                &required_inputs,
                card.get("execution_chains").and_then(Value::as_array),
            );
            normalize_protocol_capability_entry(entry);
        }
        let protocols = per_protocol.into_values().collect::<Vec<_>>();
        let protocol_count = protocols.len();
        let topic_count = global_topics.len();
        json!({
            "schema":"ais-agent-capability-view/0.0.2",
            "ready": !protocols.is_empty(),
            "protocols": protocols,
            "topics": global_topics.into_iter().collect::<Vec<_>>(),
            "counts": {
                "protocols": protocol_count,
                "actions": self.executable_candidates.actions.len(),
                "queries": self.executable_candidates.queries.len(),
                "topics": topic_count,
            }
        })
    }

    fn required_inputs_for_ref(&self, reference: &str) -> Vec<String> {
        let Some(detail) = self.detail_by_ref.get(reference) else {
            return Vec::new();
        };
        let Some(params) = detail.get("params").and_then(Value::as_array) else {
            return Vec::new();
        };
        let mut out = params
            .iter()
            .filter(|item| item.get("required").and_then(Value::as_bool) == Some(true))
            .filter_map(|item| item.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Debug, Clone, Default)]
struct CandidateSemanticHints {
    declared_topics: Vec<String>,
    risk_tags: Vec<String>,
    protocol_tags: Vec<String>,
}

fn build_semantic_hints_by_ref(
    protocols: &[ProtocolDocument],
) -> BTreeMap<String, CandidateSemanticHints> {
    let mut out = BTreeMap::<String, CandidateSemanticHints>::new();
    for protocol in protocols {
        let Some(schema_name) = protocol_schema_name(protocol) else {
            continue;
        };
        let protocol_tags = protocol
            .meta
            .as_object()
            .and_then(|meta| meta.get("tags"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (id, spec) in &protocol.actions {
            let reference = format!("{schema_name}/{id}");
            out.insert(
                reference,
                CandidateSemanticHints {
                    declared_topics: extract_declared_topics(spec),
                    risk_tags: extract_string_array(spec.get("risk_tags")),
                    protocol_tags: protocol_tags.clone(),
                },
            );
        }
        for (id, spec) in &protocol.queries {
            let reference = format!("{schema_name}/{id}");
            out.insert(
                reference,
                CandidateSemanticHints {
                    declared_topics: extract_declared_topics(spec),
                    risk_tags: extract_string_array(spec.get("risk_tags")),
                    protocol_tags: protocol_tags.clone(),
                },
            );
        }
    }
    out
}

fn protocol_schema_name(protocol: &ProtocolDocument) -> Option<String> {
    let meta = protocol.meta.as_object()?;
    let protocol_id = meta.get("protocol").and_then(Value::as_str)?;
    let version = meta.get("version").and_then(Value::as_str)?;
    Some(format!("{protocol_id}@{version}"))
}

fn extract_declared_topics(spec: &Value) -> Vec<String> {
    let Some(extensions) = spec.get("extensions") else {
        return Vec::new();
    };
    let mut out = Vec::<String>::new();
    for pointer in [
        "/agent/topic",
        "/agent/topics",
        "/semantic/topic",
        "/semantic/topics",
        "/planning/topic",
        "/planning/topics",
        "/topic",
        "/topics",
    ] {
        if let Some(value) = pointer_lookup(extensions, pointer) {
            match value {
                Value::String(single) => {
                    if let Some(topic) = normalize_topic(single) {
                        out.push(topic);
                    }
                }
                Value::Array(items) => {
                    for item in items.iter().filter_map(Value::as_str) {
                        if let Some(topic) = normalize_topic(item) {
                            out.push(topic);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut seen = BTreeSet::<String>::new();
    out.retain(|item| seen.insert(item.clone()));
    out
}

fn pointer_lookup<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in pointer.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        let key = segment.replace("~1", "/").replace("~0", "~");
        let object = current.as_object()?;
        current = object.get(key.as_str())?;
    }
    Some(current)
}

fn extract_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn merge_unique_strings(entry: &mut Value, key: &str, values: &[Value]) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(existing) = slot.as_array_mut() else {
        return;
    };
    let mut set = existing
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    for value in values.iter().filter_map(Value::as_str) {
        if set.insert(value.to_string()) {
            existing.push(Value::String(value.to_string()));
        }
    }
}

fn push_unique_named_item(
    entry: &mut Value,
    key: &str,
    name: &str,
    reference: &str,
    required_inputs: &[String],
    topic: &str,
) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(items) = slot.as_array_mut() else {
        return;
    };
    if items
        .iter()
        .any(|item| item.get("ref").and_then(Value::as_str) == Some(reference))
    {
        return;
    }
    items.push(json!({
        "name": name,
        "ref": reference,
        "required_inputs": required_inputs,
        "topic": topic,
    }));
}

fn merge_unique_required_inputs(entry: &mut Value, required_inputs: &[String]) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry("required_inputs".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(items) = slot.as_array_mut() else {
        return;
    };
    let mut set = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    for value in required_inputs {
        if set.insert(value.clone()) {
            items.push(Value::String(value.clone()));
        }
    }
}

fn merge_unique_protocol_topic(entry: &mut Value, topic: &str) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry("topics".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(items) = slot.as_array_mut() else {
        return;
    };
    if items
        .iter()
        .any(|item| item.as_str().is_some_and(|value| value == topic))
    {
        return;
    }
    items.push(Value::String(topic.to_string()));
}

fn merge_topic_card(
    entry: &mut Value,
    topic: &str,
    kind: &str,
    reference: &str,
    required_inputs: &[String],
    chains: Option<&Vec<Value>>,
) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry("topic_cards".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(cards) = slot.as_array_mut() else {
        return;
    };
    let card_index = cards
        .iter()
        .position(|item| item.get("topic").and_then(Value::as_str) == Some(topic));
    let target = match card_index {
        Some(index) => cards.get_mut(index),
        None => {
            cards.push(json!({
                "topic": topic,
                "actions": Vec::<String>::new(),
                "queries": Vec::<String>::new(),
                "required_inputs": Vec::<String>::new(),
                "chains": Vec::<String>::new(),
            }));
            cards.last_mut()
        }
    };
    let Some(target) = target else {
        return;
    };
    let slot_name = if kind == "action" {
        "actions"
    } else {
        "queries"
    };
    merge_unique_strings_from_iter(target, slot_name, std::iter::once(reference.to_string()));
    merge_unique_strings_from_iter(target, "required_inputs", required_inputs.iter().cloned());
    if let Some(chains) = chains {
        merge_unique_strings(target, "chains", chains);
    }
}

fn merge_unique_strings_from_iter<I>(entry: &mut Value, key: &str, values: I)
where
    I: IntoIterator<Item = String>,
{
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    let slot = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    let Some(items) = slot.as_array_mut() else {
        return;
    };
    let mut set = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    for value in values {
        if set.insert(value.clone()) {
            items.push(Value::String(value));
        }
    }
}

fn normalize_protocol_capability_entry(entry: &mut Value) {
    for key in ["chains", "required_inputs", "topics"] {
        if let Some(items) = entry.get_mut(key).and_then(Value::as_array_mut) {
            items.sort_by(|left, right| {
                left.as_str()
                    .unwrap_or_default()
                    .cmp(right.as_str().unwrap_or_default())
            });
        }
    }
    for key in ["actions", "queries"] {
        if let Some(items) = entry.get_mut(key).and_then(Value::as_array_mut) {
            items.sort_by(|left, right| {
                left.get("ref")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(right.get("ref").and_then(Value::as_str).unwrap_or_default())
            });
        }
    }
    if let Some(cards) = entry.get_mut("topic_cards").and_then(Value::as_array_mut) {
        cards.sort_by(|left, right| {
            left.get("topic")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("topic")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        });
        for card in cards {
            for key in ["actions", "queries", "required_inputs", "chains"] {
                if let Some(items) = card.get_mut(key).and_then(Value::as_array_mut) {
                    items.sort_by(|left, right| {
                        left.as_str()
                            .unwrap_or_default()
                            .cmp(right.as_str().unwrap_or_default())
                    });
                }
            }
        }
    }
}

fn semantic_topic_for_candidate(
    kind: &str,
    declared_topics: &[String],
    risk_tags: &[String],
    protocol_tags: &[String],
) -> String {
    if let Some(topic) = declared_topics.first() {
        return topic.clone();
    }
    if let Some(topic) = risk_tags
        .iter()
        .find_map(|tag| normalize_topic(tag).map(|value| format!("{kind}.{value}")))
    {
        return topic;
    }
    if let Some(topic) = protocol_tags
        .iter()
        .find_map(|tag| normalize_topic(tag).map(|value| format!("{kind}.{value}")))
    {
        return topic;
    }
    format!("{kind}.general")
}

fn normalize_topic(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(value.len());
    let mut prev_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            out.push(ch);
            prev_dash = false;
            continue;
        }
        if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let normalized = out.trim_matches('-').to_string();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

pub fn build_candidate_context_for_agent(
    command: &AgentCommand,
    pack: Option<&PackDocument>,
    max_index_candidates: usize,
) -> Result<Option<CandidateContext>, RunnerError> {
    let Some(workspace_root) = command.workspace.as_ref() else {
        return Ok(None);
    };

    let loaded = load_workspace_documents(workspace_root)
        .map_err(|issues| RunnerError::WorkspaceLoad(format!("{issues:?}")))?;
    if loaded.protocols.is_empty() {
        return Ok(None);
    }

    let index_catalog = build_catalog(
        CatalogBuildInput {
            protocols: &loaded.protocols,
            packs: &loaded.packs,
            workflows: &loaded.workflows,
        },
        &CatalogBuildOptions {
            created_at: None,
            card_level: CatalogCardLevel::Index,
        },
    )
    .map_err(RunnerError::from)?;
    let index = build_catalog_index(&index_catalog);
    let executable_candidates =
        get_executable_candidates(&index, pack, None, None, None).map_err(RunnerError::from)?;
    let allowed_refs = collect_candidate_refs(&executable_candidates);

    let mut index_candidates = executable_candidates.clone();
    truncate_candidates(&mut index_candidates, max_index_candidates);

    let detail_catalog = build_catalog(
        CatalogBuildInput {
            protocols: &loaded.protocols,
            packs: &loaded.packs,
            workflows: &loaded.workflows,
        },
        &CatalogBuildOptions {
            created_at: None,
            card_level: CatalogCardLevel::Detail,
        },
    )
    .map_err(RunnerError::from)?;
    let mut detail_by_ref =
        build_detail_lookup_from_catalog(detail_catalog.actions, detail_catalog.queries);
    detail_by_ref.retain(|reference, _| allowed_refs.contains(reference));

    Ok(Some(CandidateContext {
        index_candidates: json!({
            "schema": index_candidates.schema,
            "level": "name_only",
            "hash": index_candidates.hash,
            "catalog_schema": index_candidates.catalog_schema,
            "catalog_hash": index_candidates.catalog_hash,
            "actions": index_candidates.actions.iter().map(|card| to_discovery_card(card, "action")).collect::<Vec<_>>(),
            "queries": index_candidates.queries.iter().map(|card| to_discovery_card(card, "query")).collect::<Vec<_>>(),
            "execution_plugins": index_candidates.execution_plugins,
        }),
        executable_candidates,
        protocols: loaded.protocols,
        detail_by_ref,
    }))
}

fn collect_candidate_refs(candidates: &ExecutableCandidates) -> BTreeSet<String> {
    candidates
        .actions
        .iter()
        .chain(candidates.queries.iter())
        .filter_map(|card| card.get("ref").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn truncate_candidates(candidates: &mut ExecutableCandidates, max_index_candidates: usize) {
    if max_index_candidates == 0 {
        candidates.actions.clear();
        candidates.queries.clear();
        return;
    }
    let action_len = candidates.actions.len();
    if action_len >= max_index_candidates {
        candidates.actions.truncate(max_index_candidates);
        candidates.queries.clear();
        return;
    }
    let remaining = max_index_candidates.saturating_sub(action_len);
    candidates.queries.truncate(remaining);
}

fn build_detail_lookup_from_catalog(
    actions: Vec<Value>,
    queries: Vec<Value>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::<String, Value>::new();
    for action in actions {
        if let Some(reference) = action.get("ref").and_then(Value::as_str) {
            out.insert(reference.to_string(), normalize_action_detail(action));
        }
    }
    for query in queries {
        if let Some(reference) = query.get("ref").and_then(Value::as_str) {
            out.insert(reference.to_string(), normalize_query_detail(query));
        }
    }
    out
}

fn normalize_action_detail(action: Value) -> Value {
    json!({
        "kind": "action",
        "ref": action.get("ref").and_then(Value::as_str).unwrap_or_default(),
        "id": action.get("id").and_then(Value::as_str).unwrap_or_default(),
        "description": action.get("description").cloned(),
        "risk_level": action.get("risk_level").cloned(),
        "risk_tags": action.get("risk_tags").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "params": action.get("params").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "returns": action.get("returns").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "requires_queries": action.get("requires_queries").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "write_gate": action
            .get("write_gate")
            .cloned()
            .or_else(|| action.pointer("/extensions/write_gate").cloned()),
        "execution_types": action.get("execution_types").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "execution_chains": action.get("execution_chains").cloned().unwrap_or_else(|| Value::Array(vec![])),
    })
}

fn normalize_query_detail(query: Value) -> Value {
    json!({
        "kind": "query",
        "ref": query.get("ref").and_then(Value::as_str).unwrap_or_default(),
        "id": query.get("id").and_then(Value::as_str).unwrap_or_default(),
        "description": query.get("description").cloned(),
        "params": query.get("params").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "returns": query.get("returns").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "execution_types": query.get("execution_types").cloned().unwrap_or_else(|| Value::Array(vec![])),
        "execution_chains": query.get("execution_chains").cloned().unwrap_or_else(|| Value::Array(vec![])),
    })
}

fn to_discovery_card(card: &Value, kind: &str) -> Value {
    let reference = card
        .get("ref")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut compact = Map::<String, Value>::new();
    compact.insert("ref".to_string(), Value::String(reference));
    compact.insert("kind".to_string(), Value::String(kind.to_string()));

    if let Some(risk_level) = card.get("risk_level") {
        compact.insert("risk_level".to_string(), risk_level.clone());
    }
    if let Some(chains) = card.get("execution_chains").and_then(Value::as_array) {
        let mut compact_chains = chains
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        compact_chains.sort();
        compact_chains.dedup();
        compact_chains.truncate(4);
        if !compact_chains.is_empty() {
            compact.insert(
                "chains".to_string(),
                Value::Array(compact_chains.into_iter().map(Value::String).collect()),
            );
        }
    }

    Value::Object(compact)
}

fn matches_keyword(card: &Value, query_tokens: Option<&[String]>) -> bool {
    let Some(query_tokens) = query_tokens else {
        return true;
    };
    if query_tokens.is_empty() {
        return true;
    }

    let ref_text = card.get("ref").and_then(Value::as_str).unwrap_or_default();
    let id_text = card.get("id").and_then(Value::as_str).unwrap_or_default();
    let desc_text = card
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut haystacks = vec![
        normalize_search_text(ref_text),
        normalize_search_text(id_text),
        normalize_search_text(desc_text),
    ];
    let risk_haystacks = card
        .get("risk_tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(normalize_search_text)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    haystacks.extend(risk_haystacks);
    query_tokens.iter().all(|token| {
        haystacks
            .iter()
            .any(|haystack| haystack.contains(token.as_str()))
    })
}

fn normalize_search_query_tokens(value: &str) -> Vec<String> {
    let stop_words = ["a", "an", "the", "for", "on", "to", "my", "me", "please"];
    let mut tokens = tokenize_search_text(value)
        .into_iter()
        .map(canonicalize_search_token)
        .filter(|token| !token.is_empty())
        .filter(|token| !stop_words.contains(&token.as_str()))
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn normalize_search_text(value: &str) -> String {
    tokenize_search_text(value)
        .into_iter()
        .map(canonicalize_search_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize_search_text(value: &str) -> Vec<String> {
    let lowered = value
        .to_ascii_lowercase()
        .replace("erc-20", "erc20")
        .replace("erc 20", "erc20")
        .replace("balanceof", "balance");
    lowered
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn canonicalize_search_token(token: String) -> String {
    match token.as_str() {
        "erc20" | "token" | "tokens" => "token".to_string(),
        "native" | "eth" => "native".to_string(),
        "balanceof" | "balance" | "balances" => "balance".to_string(),
        "transfer" | "send" | "payment" => "transfer".to_string(),
        _ => token,
    }
}

fn matches_chain(card: &Value, chain: Option<&str>) -> bool {
    let Some(chain) = chain else {
        return true;
    };
    let Some(chains) = card.get("execution_chains").and_then(Value::as_array) else {
        return false;
    };
    chains
        .iter()
        .filter_map(Value::as_str)
        .any(|pattern| chain_pattern_matches(pattern, chain))
}

fn chain_pattern_matches(pattern: &str, chain: &str) -> bool {
    if pattern == "*" || pattern.eq_ignore_ascii_case(chain) {
        return true;
    }
    if let Some(namespace) = pattern.strip_suffix("*") {
        return chain.starts_with(namespace);
    }
    false
}

fn matches_risk(card: &Value, min: Option<u8>, max: Option<u8>, is_action: bool) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }
    if !is_action {
        return false;
    }
    let Some(level) = read_risk_level(card) else {
        return false;
    };
    if let Some(min_level) = min {
        if level < min_level {
            return false;
        }
    }
    if let Some(max_level) = max {
        if level > max_level {
            return false;
        }
    }
    true
}

fn read_risk_level(card: &Value) -> Option<u8> {
    if let Some(level) = card.get("risk_level").and_then(Value::as_u64) {
        return u8::try_from(level).ok();
    }
    card.get("risk_level")
        .and_then(Value::as_str)
        .and_then(|level| level.parse::<u8>().ok())
}

#[cfg(test)]
#[path = "tests/candidates_module.rs"]
mod tests;
