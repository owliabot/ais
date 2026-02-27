use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningScope {
    snapshot_hash: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanningMemoryBudget {
    pub max_entries: usize,
    pub max_entry_chars: usize,
    pub max_total_chars: usize,
}

impl Default for PlanningMemoryBudget {
    fn default() -> Self {
        Self {
            max_entries: 48,
            max_entry_chars: 8_000,
            max_total_chars: 120_000,
        }
    }
}

const TOOL_MEMORY_MIN_TOKENS: usize = 1200;
const TOOL_MEMORY_MAX_TOKENS: usize = 6000;
const TOOL_MEMORY_DEFAULT_TOKENS: usize = 2400;
const TOOL_MEMORY_MAX_LIST_INVENTORY_ENTRIES: usize = 2;
const TOOL_MEMORY_MAX_CATALOG_ENTRIES: usize = 6;
const TOOL_MEMORY_MAX_DETAIL_ENTRIES: usize = 6;
const TOOL_MEMORY_MAX_GUIDE_ENTRIES: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct GuideProjection {
    #[serde(default)]
    schema: BTreeMap<String, Value>,
    #[serde(default)]
    topic: BTreeMap<String, Value>,
}

impl GuideProjection {
    fn is_empty(&self) -> bool {
        self.schema.is_empty() && self.topic.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuideEntryKind {
    Schema,
    Topic,
}

impl GuideEntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Topic => "topic",
        }
    }
}

#[derive(Debug, Clone)]
struct GuideSummaryEntry {
    kind: GuideEntryKind,
    id: String,
    recency_rank: usize,
    mode: Option<String>,
    defs: Vec<String>,
    summary: Option<String>,
    error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlanningMemorySnapshot {
    pub snapshot_hash: String,
    #[serde(default)]
    pub tool_cache: Vec<PlanningMemoryCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlanningMemoryCacheEntry {
    pub key: String,
    pub content: String,
}

#[derive(Debug, Default)]
pub(crate) struct PlanningMemory {
    scope: Option<PlanningScope>,
    tool_cache: HashMap<String, String>,
    order: VecDeque<String>,
}

impl PlanningMemory {
    pub(crate) fn ensure_scope(&mut self, _session_id: &str, snapshot_hash: &str) {
        let next = PlanningScope {
            snapshot_hash: snapshot_hash.to_string(),
        };
        if self.scope.as_ref() == Some(&next) {
            return;
        }
        self.scope = Some(next);
        self.tool_cache.clear();
        self.order.clear();
    }

    pub(crate) fn clear(&mut self) {
        self.scope = None;
        self.tool_cache.clear();
        self.order.clear();
    }

    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.tool_cache.get(key).map(String::as_str)
    }

    pub(crate) fn insert(&mut self, key: String, content: String) {
        self.insert_with_budget(key, content, PlanningMemoryBudget::default());
    }

    pub(crate) fn insert_with_budget(
        &mut self,
        key: String,
        mut content: String,
        budget: PlanningMemoryBudget,
    ) {
        if content.chars().count() > budget.max_entry_chars {
            content = content.chars().take(budget.max_entry_chars).collect();
        }
        if self.tool_cache.contains_key(&key) {
            self.order.retain(|existing| existing != &key);
        }
        self.tool_cache.insert(key.clone(), content);
        self.order.push_back(key);
        self.enforce_budget(budget);
    }

    pub(crate) fn checkpoint_snapshot(
        &self,
        budget: PlanningMemoryBudget,
    ) -> Option<PlanningMemorySnapshot> {
        let snapshot_hash = self.scope.as_ref()?.snapshot_hash.clone();
        let mut remaining = budget.max_total_chars;
        let mut entries = Vec::<PlanningMemoryCacheEntry>::new();
        for key in &self.order {
            let Some(content) = self.tool_cache.get(key) else {
                continue;
            };
            if key.is_empty() || content.is_empty() {
                continue;
            }
            let entry_chars = content.chars().count();
            if entry_chars > budget.max_entry_chars {
                continue;
            }
            if entry_chars > remaining {
                break;
            }
            remaining = remaining.saturating_sub(entry_chars);
            entries.push(PlanningMemoryCacheEntry {
                key: key.clone(),
                content: content.clone(),
            });
            if entries.len() >= budget.max_entries {
                break;
            }
        }
        if entries.is_empty() {
            return None;
        }
        Some(PlanningMemorySnapshot {
            snapshot_hash,
            tool_cache: entries,
        })
    }

    pub(crate) fn restore_from_checkpoint(
        &mut self,
        value: &Value,
        budget: PlanningMemoryBudget,
    ) -> bool {
        let Ok(snapshot) = serde_json::from_value::<PlanningMemorySnapshot>(value.clone()) else {
            return false;
        };
        if snapshot.snapshot_hash.trim().is_empty() {
            return false;
        }
        self.scope = Some(PlanningScope {
            snapshot_hash: snapshot.snapshot_hash,
        });
        self.tool_cache.clear();
        self.order.clear();
        for entry in snapshot.tool_cache {
            if entry.key.trim().is_empty() || entry.content.is_empty() {
                continue;
            }
            self.insert_with_budget(entry.key, entry.content, budget);
        }
        !self.tool_cache.is_empty()
    }

    pub(crate) fn checkpoint_value(&self, budget: PlanningMemoryBudget) -> Option<Value> {
        let snapshot = self.checkpoint_snapshot(budget)?;
        serde_json::to_value(snapshot).ok()
    }

    pub(crate) fn tool_memory_projection(&self, max_tokens: usize) -> Option<Value> {
        let snapshot_hash = self.scope.as_ref()?.snapshot_hash.clone();
        let token_budget = normalize_tool_memory_token_budget(max_tokens);
        let mut list_inventory_raw = Vec::<(usize, Value)>::new();
        let mut catalog_search_raw = Vec::<(usize, Value)>::new();
        let mut candidate_detail_raw = Vec::<(usize, Value)>::new();
        let mut guide_raw = Vec::<GuideSummaryEntry>::new();

        for (recency_rank, key) in self.order.iter().rev().enumerate() {
            let Some(content) = self.tool_cache.get(key) else {
                continue;
            };
            let tool_name = key.split(':').next().unwrap_or_default();
            match tool_name {
                "list_candidates" => {
                    if let Some(entry) = summarize_list_candidates(content.as_str()) {
                        list_inventory_raw.push((recency_rank, entry));
                    }
                }
                "catalog.search" => {
                    if let Some(entry) = summarize_catalog_search(content.as_str()) {
                        catalog_search_raw.push((recency_rank, entry));
                    }
                }
                "get_candidate_detail" => {
                    if let Some(entry) = summarize_candidate_detail(content.as_str()) {
                        candidate_detail_raw.push((recency_rank, entry));
                    }
                }
                "guide.get" => {
                    if let Some(entry) = summarize_guide_get(content.as_str(), recency_rank) {
                        guide_raw.push(entry);
                    }
                }
                _ => {}
            }
        }

        let list_inventory = select_list_inventory_entries(list_inventory_raw);
        let catalog_search = select_catalog_entries(catalog_search_raw);
        let candidate_detail = select_candidate_detail_entries(candidate_detail_raw);
        let guide = select_guide_entries(guide_raw);

        if list_inventory.is_empty()
            && catalog_search.is_empty()
            && candidate_detail.is_empty()
            && guide.is_empty()
        {
            return None;
        }

        let mut projection = json!({
            "schema": "ais-agent-tool-memory-projection/0.0.1",
            "snapshot_hash": snapshot_hash,
            "recent": {
                "list_inventory": list_inventory,
                "catalog_search": catalog_search,
                "candidate_detail": candidate_detail,
                "guide": {
                    "schema": guide.schema,
                    "topic": guide.topic,
                },
            },
            "hint": "Use this memory first; call discovery/schema tools only when missing or stale."
        });
        trim_tool_memory_projection_to_budget(&mut projection, token_budget);
        if tool_memory_projection_empty(&projection) {
            return None;
        }
        let estimated_tokens = estimate_tokens_json(&projection);
        if let Some(object) = projection.as_object_mut() {
            object.insert(
                "token_budget".to_string(),
                Value::Number(u64::try_from(token_budget).unwrap_or(0).into()),
            );
            object.insert(
                "estimated_tokens".to_string(),
                Value::Number(u64::try_from(estimated_tokens).unwrap_or(0).into()),
            );
            object.insert(
                "estimator".to_string(),
                Value::String("chars_div_4".to_string()),
            );
        }
        Some(projection)
    }

    fn enforce_budget(&mut self, budget: PlanningMemoryBudget) {
        while self.tool_cache.len() > budget.max_entries {
            self.evict_oldest();
        }
        while self.total_chars() > budget.max_total_chars {
            self.evict_oldest();
        }
    }

    fn total_chars(&self) -> usize {
        self.tool_cache
            .values()
            .map(|value| value.chars().count())
            .sum()
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest) = self.order.pop_front() {
            self.tool_cache.remove(oldest.as_str());
        }
    }
}

fn normalize_tool_memory_token_budget(max_tokens: usize) -> usize {
    let requested = if max_tokens == 0 {
        TOOL_MEMORY_DEFAULT_TOKENS
    } else {
        max_tokens
    };
    requested.clamp(TOOL_MEMORY_MIN_TOKENS, TOOL_MEMORY_MAX_TOKENS)
}

fn summarize_catalog_search(content: &str) -> Option<Value> {
    let payload = serde_json::from_str::<Value>(content).ok()?;
    let mut entry = Map::<String, Value>::new();
    if let Some(query) = payload.get("query").and_then(Value::as_str) {
        if !query.trim().is_empty() {
            entry.insert("query".to_string(), Value::String(query.to_string()));
        }
    }
    if let Some(filters) = payload.get("filters").and_then(Value::as_object) {
        let mut compact_filters = Map::<String, Value>::new();
        for key in ["kind", "chain", "min_risk_level", "max_risk_level"] {
            if let Some(value) = filters.get(key) {
                compact_filters.insert(key.to_string(), value.clone());
            }
        }
        if !compact_filters.is_empty() {
            entry.insert("filters".to_string(), Value::Object(compact_filters));
        }
    }
    if let Some(returned) = payload.get("returned_matches").and_then(Value::as_u64) {
        entry.insert(
            "returned_matches".to_string(),
            Value::Number(returned.into()),
        );
    }

    let top_refs = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(6)
                .filter_map(|item| {
                    let reference = item.get("ref").and_then(Value::as_str)?;
                    let mut card = Map::<String, Value>::new();
                    card.insert("ref".to_string(), Value::String(reference.to_string()));
                    if let Some(kind) = item.get("kind").and_then(Value::as_str) {
                        card.insert("kind".to_string(), Value::String(kind.to_string()));
                    }
                    if let Some(protocol) = item
                        .get("schema_name")
                        .or_else(|| item.get("protocol"))
                        .and_then(Value::as_str)
                    {
                        card.insert("protocol".to_string(), Value::String(protocol.to_string()));
                    } else if let Some((protocol, _)) = reference.split_once('/') {
                        card.insert("protocol".to_string(), Value::String(protocol.to_string()));
                    }
                    Some(Value::Object(card))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !top_refs.is_empty() {
        entry.insert("top_refs".to_string(), Value::Array(top_refs));
    }

    if let Some(hint) = payload.get("hint").and_then(Value::as_object) {
        let mut compact_hint = Map::<String, Value>::new();
        for key in ["reason_code", "next_tool", "message"] {
            if let Some(value) = hint.get(key) {
                compact_hint.insert(key.to_string(), value.clone());
            }
        }
        if !compact_hint.is_empty() {
            entry.insert("hint".to_string(), Value::Object(compact_hint));
        }
    }

    Some(Value::Object(entry))
}

fn summarize_list_candidates(content: &str) -> Option<Value> {
    let payload = serde_json::from_str::<Value>(content).ok()?;
    let protocols = payload.get("protocols").and_then(Value::as_array)?;
    let mut compact_protocols = Vec::<Value>::new();
    let mut action_count = 0usize;
    let mut query_count = 0usize;

    for protocol in protocols.iter().take(8) {
        let Some(protocol_name) = protocol.get("protocol").and_then(Value::as_str) else {
            continue;
        };
        let actions = protocol
            .get("actions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("ref").and_then(Value::as_str))
                    .take(8)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let queries = protocol
            .get("queries")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("ref").and_then(Value::as_str))
                    .take(8)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        action_count = action_count.saturating_add(actions.len());
        query_count = query_count.saturating_add(queries.len());

        let chains = protocol
            .get("chains")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .take(4)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        compact_protocols.push(json!({
            "protocol": protocol_name,
            "chains": chains,
            "actions": actions,
            "queries": queries,
        }));
    }

    if compact_protocols.is_empty() {
        return None;
    }

    let protocol_count = compact_protocols.len();
    Some(json!({
        "protocols": compact_protocols,
        "counts": {
            "protocols": protocol_count,
            "actions": action_count,
            "queries": query_count,
        }
    }))
}

fn summarize_candidate_detail(content: &str) -> Option<Value> {
    let payload = serde_json::from_str::<Value>(content).ok()?;
    let details = payload.get("details").and_then(Value::as_array)?;
    let mut signatures = Vec::<Value>::new();
    for detail in details.iter().take(6) {
        let reference = detail.get("ref").and_then(Value::as_str)?;
        let mut signature = Map::<String, Value>::new();
        signature.insert("ref".to_string(), Value::String(reference.to_string()));
        if let Some(kind) = detail.get("kind").and_then(Value::as_str) {
            signature.insert("kind".to_string(), Value::String(kind.to_string()));
        }

        let required_inputs = detail
            .get("params")
            .and_then(Value::as_array)
            .map(|params| {
                let mut names = params
                    .iter()
                    .filter(|param| {
                        param
                            .get("required")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                    .filter_map(|param| param.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                names.sort();
                names.dedup();
                names
            })
            .unwrap_or_default();
        signature.insert(
            "required_inputs".to_string(),
            Value::Array(
                required_inputs
                    .iter()
                    .map(|value| Value::String(value.clone()))
                    .collect(),
            ),
        );

        if let Some(chains) = detail.get("execution_chains").and_then(Value::as_array) {
            let mut compact = chains
                .iter()
                .take(4)
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            compact.sort();
            compact.dedup();
            signature.insert(
                "chains".to_string(),
                Value::Array(compact.into_iter().map(Value::String).collect()),
            );
        }
        signatures.push(Value::Object(signature));
    }
    if signatures.is_empty() {
        return None;
    }
    Some(json!({ "signatures": signatures }))
}

fn summarize_guide_get(content: &str, recency_rank: usize) -> Option<GuideSummaryEntry> {
    let payload = serde_json::from_str::<Value>(content).ok()?;
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error_code = payload
        .pointer("/error/code")
        .and_then(Value::as_str)
        .map(str::to_string);
    if kind == "schema" {
        let schema_id = payload
            .pointer("/schema/id")
            .and_then(Value::as_str)?
            .to_string();
        let mode = payload
            .pointer("/schema/mode")
            .and_then(Value::as_str)
            .map(str::to_string);
        let defs = payload
            .pointer("/schema/digest/defs")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .take(8)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                payload
                    .pointer("/schema/json/$defs")
                    .and_then(Value::as_object)
                    .map(|map| first_sorted_keys(map, 8))
            })
            .unwrap_or_default();
        return Some(GuideSummaryEntry {
            kind: GuideEntryKind::Schema,
            id: schema_id,
            recency_rank,
            mode,
            defs,
            summary: None,
            error_code,
        });
    }
    if kind == "topic" {
        let topic_id = payload
            .pointer("/topic/topic")
            .and_then(Value::as_str)
            .or_else(|| payload.pointer("/topic/requested").and_then(Value::as_str))?
            .to_string();
        let summary = payload
            .pointer("/topic/summary")
            .and_then(Value::as_str)
            .map(|text| clip_string(text, 200));
        return Some(GuideSummaryEntry {
            kind: GuideEntryKind::Topic,
            id: topic_id,
            recency_rank,
            mode: None,
            defs: Vec::new(),
            summary,
            error_code,
        });
    }
    None
}

fn select_list_inventory_entries(raw: Vec<(usize, Value)>) -> Vec<Value> {
    let mut output = Vec::<Value>::new();
    let mut seen_signatures = BTreeSet::<String>::new();
    for (_, entry) in raw {
        let signature = serde_json::to_string(&entry).unwrap_or_default();
        if !seen_signatures.insert(signature) {
            continue;
        }
        output.push(entry);
        if output.len() >= TOOL_MEMORY_MAX_LIST_INVENTORY_ENTRIES {
            break;
        }
    }
    output
}

fn select_catalog_entries(raw: Vec<(usize, Value)>) -> Vec<Value> {
    let mut output = Vec::<Value>::new();
    let mut seen_signatures = BTreeSet::<String>::new();
    let mut seen_refs = BTreeSet::<String>::new();

    for (_, mut entry) in raw {
        let signature = catalog_entry_signature(&entry);
        if !seen_signatures.insert(signature) {
            continue;
        }
        dedupe_entry_top_refs(&mut entry, &mut seen_refs);
        let has_top_refs = entry
            .get("top_refs")
            .and_then(Value::as_array)
            .is_some_and(|refs| !refs.is_empty());
        let returned_matches = entry
            .get("returned_matches")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if !has_top_refs && returned_matches == 0 {
            continue;
        }
        output.push(entry);
        if output.len() >= TOOL_MEMORY_MAX_CATALOG_ENTRIES {
            break;
        }
    }
    output
}

fn catalog_entry_signature(entry: &Value) -> String {
    let query = entry
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let filters = entry
        .get("filters")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let normalized_filters = serde_json::to_string(&filters).unwrap_or_else(|_| "{}".to_string());
    format!("{query}|{normalized_filters}")
}

fn dedupe_entry_top_refs(entry: &mut Value, seen_refs: &mut BTreeSet<String>) {
    let Some(items) = entry.get_mut("top_refs").and_then(Value::as_array_mut) else {
        return;
    };
    let mut deduped = Vec::<Value>::new();
    for item in items.iter() {
        let Some(reference) = item.get("ref").and_then(Value::as_str) else {
            continue;
        };
        if seen_refs.insert(reference.to_string()) {
            deduped.push(item.clone());
        }
    }
    *items = deduped;
}

fn select_candidate_detail_entries(raw: Vec<(usize, Value)>) -> Vec<Value> {
    let mut output = Vec::<Value>::new();
    let mut seen_refs = BTreeSet::<String>::new();
    for (_, mut entry) in raw {
        dedupe_detail_signatures(&mut entry, &mut seen_refs);
        let has_signatures = entry
            .get("signatures")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty());
        if !has_signatures {
            continue;
        }
        output.push(entry);
        if output.len() >= TOOL_MEMORY_MAX_DETAIL_ENTRIES {
            break;
        }
    }
    output
}

fn dedupe_detail_signatures(entry: &mut Value, seen_refs: &mut BTreeSet<String>) {
    let Some(items) = entry.get_mut("signatures").and_then(Value::as_array_mut) else {
        return;
    };
    let mut deduped = Vec::<Value>::new();
    for item in items.iter() {
        let Some(reference) = item.get("ref").and_then(Value::as_str) else {
            continue;
        };
        if seen_refs.insert(reference.to_string()) {
            deduped.push(item.clone());
        }
    }
    *items = deduped;
}

fn select_guide_entries(raw: Vec<GuideSummaryEntry>) -> GuideProjection {
    let mut deduped = BTreeMap::<String, GuideSummaryEntry>::new();
    for entry in raw {
        let key = format!("{}:{}", entry.kind.as_str(), entry.id);
        match deduped.get_mut(key.as_str()) {
            Some(existing) => {
                if should_replace_guide_entry(existing, &entry) {
                    *existing = entry;
                }
            }
            None => {
                deduped.insert(key, entry);
            }
        }
    }

    let mut selected = deduped.into_values().collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        let left_priority = guide_entry_priority(left.kind, left.id.as_str());
        let right_priority = guide_entry_priority(right.kind, right.id.as_str());
        right_priority
            .cmp(&left_priority)
            .then_with(|| left.recency_rank.cmp(&right.recency_rank))
    });
    selected.truncate(TOOL_MEMORY_MAX_GUIDE_ENTRIES);

    let mut projection = GuideProjection::default();
    for entry in selected {
        let mut payload = Map::<String, Value>::new();
        if let Some(mode) = entry.mode {
            payload.insert("mode".to_string(), Value::String(mode));
        }
        if !entry.defs.is_empty() {
            payload.insert(
                "defs".to_string(),
                Value::Array(entry.defs.into_iter().map(Value::String).collect()),
            );
        }
        if let Some(summary) = entry.summary {
            payload.insert("summary".to_string(), Value::String(summary));
        }
        if let Some(error_code) = entry.error_code {
            payload.insert("error_code".to_string(), Value::String(error_code));
        }
        let value = Value::Object(payload);
        match entry.kind {
            GuideEntryKind::Schema => {
                projection.schema.insert(entry.id, value);
            }
            GuideEntryKind::Topic => {
                projection.topic.insert(entry.id, value);
            }
        }
    }
    projection
}

fn should_replace_guide_entry(existing: &GuideSummaryEntry, incoming: &GuideSummaryEntry) -> bool {
    let existing_full = guide_entry_is_full_schema(existing);
    let incoming_full = guide_entry_is_full_schema(incoming);
    if incoming_full && !existing_full {
        return true;
    }
    if existing_full && !incoming_full {
        return false;
    }
    if incoming.recency_rank < existing.recency_rank {
        return true;
    }
    if incoming.recency_rank > existing.recency_rank {
        return false;
    }
    incoming.defs.len() > existing.defs.len()
}

fn guide_entry_is_full_schema(entry: &GuideSummaryEntry) -> bool {
    entry.kind == GuideEntryKind::Schema
        && entry
            .mode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("full"))
}

fn guide_entry_priority(kind: GuideEntryKind, id: &str) -> i32 {
    match (kind, id) {
        (GuideEntryKind::Schema, "ais-plan-sketch/0.1.0") => 400,
        (GuideEntryKind::Topic, "cel") => 350,
        (GuideEntryKind::Schema, "ais-agent-planning-tools/0.1.0") => 320,
        (GuideEntryKind::Topic, "valueref") => 280,
        (GuideEntryKind::Schema, _) => 200,
        (GuideEntryKind::Topic, _) => 150,
    }
}

fn first_sorted_keys(map: &Map<String, Value>, limit: usize) -> Vec<String> {
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys.truncate(limit.max(1));
    keys
}

fn clip_string(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut clipped = value.chars().take(max_chars).collect::<String>();
    clipped.push_str("...");
    clipped
}

fn trim_tool_memory_projection_to_budget(projection: &mut Value, token_budget: usize) {
    while estimate_tokens_json(projection) > token_budget {
        if !trim_tool_memory_projection_once(projection) {
            break;
        }
    }
}

fn trim_tool_memory_projection_once(projection: &mut Value) -> bool {
    let Some(recent) = projection.get_mut("recent").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut target: Option<&str> = None;
    let mut target_len = 0usize;
    let catalog_len = recent
        .get("catalog_search")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if catalog_len > target_len {
        target_len = catalog_len;
        target = Some("catalog_search");
    }
    let detail_len = recent
        .get("candidate_detail")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if detail_len > target_len {
        target_len = detail_len;
        target = Some("candidate_detail");
    }
    let list_len = recent
        .get("list_inventory")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if list_len > target_len {
        target_len = list_len;
        target = Some("list_inventory");
    }
    let guide_len = recent
        .get("guide")
        .and_then(guide_projection_entry_count)
        .unwrap_or(0);
    if guide_len > target_len {
        target_len = guide_len;
        target = Some("guide");
    }
    let Some(target_key) = target else {
        return false;
    };
    if target_len == 0 {
        return false;
    }
    match target_key {
        "list_inventory" | "catalog_search" | "candidate_detail" => recent
            .get_mut(target_key)
            .and_then(Value::as_array_mut)
            .and_then(|items| items.pop())
            .is_some(),
        "guide" => recent
            .get_mut("guide")
            .is_some_and(pop_one_guide_projection_entry),
        _ => false,
    }
}

fn tool_memory_projection_empty(projection: &Value) -> bool {
    let Some(recent) = projection.get("recent").and_then(Value::as_object) else {
        return true;
    };
    if recent
        .get("list_inventory")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    if recent
        .get("catalog_search")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    if recent
        .get("candidate_detail")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
    {
        return false;
    }
    if recent
        .get("guide")
        .and_then(guide_projection_entry_count)
        .is_some_and(|len| len > 0)
    {
        return false;
    }
    true
}

fn guide_projection_entry_count(value: &Value) -> Option<usize> {
    let object = value.as_object()?;
    let schema_len = object
        .get("schema")
        .and_then(Value::as_object)
        .map(|items| items.len())
        .unwrap_or(0);
    let topic_len = object
        .get("topic")
        .and_then(Value::as_object)
        .map(|items| items.len())
        .unwrap_or(0);
    Some(schema_len.saturating_add(topic_len))
}

fn pop_one_guide_projection_entry(value: &mut Value) -> bool {
    let target = {
        let Some(guide) = value.as_object() else {
            return false;
        };
        let mut candidates = Vec::<(i32, &'static str, String)>::new();
        if let Some(schema) = guide.get("schema").and_then(Value::as_object) {
            for id in schema.keys() {
                candidates.push((
                    guide_entry_priority(GuideEntryKind::Schema, id),
                    "schema",
                    id.clone(),
                ));
            }
        }
        if let Some(topic) = guide.get("topic").and_then(Value::as_object) {
            for id in topic.keys() {
                candidates.push((
                    guide_entry_priority(GuideEntryKind::Topic, id),
                    "topic",
                    id.clone(),
                ));
            }
        }
        candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.2.cmp(&right.2)));
        candidates.into_iter().next()
    };

    let Some((_, bucket, id)) = target else {
        return false;
    };
    value
        .as_object_mut()
        .and_then(|guide| guide.get_mut(bucket))
        .and_then(Value::as_object_mut)
        .and_then(|entries| entries.remove(id.as_str()))
        .is_some()
}

fn estimate_tokens_json(value: &Value) -> usize {
    serde_json::to_string(value)
        .ok()
        .map(|encoded| {
            let chars = encoded.chars().count();
            chars.saturating_add(3) / 4
        })
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn planning_memory_scope_is_snapshot_based() {
        let mut memory = PlanningMemory::default();
        let budget = PlanningMemoryBudget::default();
        memory.ensure_scope("session-a", "snapshot-1");
        memory.insert_with_budget("k".to_string(), "v".to_string(), budget);
        assert_eq!(memory.get("k"), Some("v"));

        memory.ensure_scope("session-b", "snapshot-1");
        assert_eq!(memory.get("k"), Some("v"));

        memory.ensure_scope("session-c", "snapshot-2");
        assert_eq!(memory.get("k"), None);
    }

    #[test]
    fn planning_memory_checkpoint_roundtrip_respects_budget() {
        let mut memory = PlanningMemory::default();
        let budget = PlanningMemoryBudget {
            max_entries: 2,
            max_entry_chars: 4,
            max_total_chars: 6,
        };
        memory.ensure_scope("s", "snap");
        memory.insert_with_budget("a".to_string(), "1111".to_string(), budget);
        memory.insert_with_budget("b".to_string(), "2222".to_string(), budget);
        memory.insert_with_budget("c".to_string(), "3333".to_string(), budget);
        let value = memory
            .checkpoint_value(budget)
            .expect("checkpoint value must exist");
        let mut restored = PlanningMemory::default();
        assert!(restored.restore_from_checkpoint(&value, budget));
        assert!(restored.get("a").is_none());
        assert!(restored.get("b").is_some() || restored.get("c").is_some());
        assert_eq!(
            serde_json::from_value::<PlanningMemorySnapshot>(value)
                .expect("snapshot")
                .snapshot_hash,
            "snap"
        );
    }

    #[test]
    fn restore_ignores_invalid_payload() {
        let mut memory = PlanningMemory::default();
        assert!(!memory.restore_from_checkpoint(&json!({"x":1}), PlanningMemoryBudget::default()));
    }

    #[test]
    fn tool_memory_projection_contains_recent_high_value_entries() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("s", "snap-1");
        memory.insert(
            "list_candidates:k0".to_string(),
            json!({
                "protocols":[
                    {
                        "protocol":"erc20@0.0.2",
                        "chains":["eip155:*"],
                        "actions":[{"ref":"erc20@0.0.2/transfer"}],
                        "queries":[{"ref":"erc20@0.0.2/balance-of"}]
                    }
                ]
            })
            .to_string(),
        );
        memory.insert(
            "catalog.search:k1".to_string(),
            json!({
                "query":"transfer",
                "returned_matches":2,
                "results":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","schema_name":"erc20@0.0.2"},
                    {"ref":"evm-native-utils@0.0.1/native-transfer","kind":"action","schema_name":"evm-native-utils@0.0.1"}
                ]
            })
            .to_string(),
        );
        memory.insert(
            "get_candidate_detail:k2".to_string(),
            json!({
                "details":[
                    {
                        "ref":"erc20@0.0.2/transfer",
                        "kind":"action",
                        "params":[
                            {"name":"to","required":true},
                            {"name":"amount","required":true},
                            {"name":"token","required":true}
                        ],
                        "execution_chains":["eip155:*"]
                    }
                ]
            })
            .to_string(),
        );
        memory.insert(
            "guide.get:k3".to_string(),
            json!({
                "kind":"topic",
                "topic":{"topic":"cel","summary":"Use deterministic CEL only."}
            })
            .to_string(),
        );

        let projection = memory
            .tool_memory_projection(1200)
            .expect("projection should exist");
        assert_eq!(
            projection.get("schema").and_then(Value::as_str),
            Some("ais-agent-tool-memory-projection/0.0.1")
        );
        assert_eq!(
            projection
                .pointer("/recent/list_inventory/0/protocols/0/protocol")
                .and_then(Value::as_str),
            Some("erc20@0.0.2")
        );
        assert_eq!(
            projection
                .pointer("/recent/catalog_search/0/top_refs/0/ref")
                .and_then(Value::as_str),
            Some("erc20@0.0.2/transfer")
        );
        assert_eq!(
            projection
                .pointer("/recent/candidate_detail/0/signatures/0/ref")
                .and_then(Value::as_str),
            Some("erc20@0.0.2/transfer")
        );
        assert_eq!(
            projection
                .pointer("/recent/guide/topic/cel/summary")
                .and_then(Value::as_str),
            Some("Use deterministic CEL only.")
        );
    }

    #[test]
    fn tool_memory_projection_budget_is_clamped_and_trimmed() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("s", "snap-2");
        for index in 0..8 {
            memory.insert(
                format!("catalog.search:k{index}"),
                json!({
                    "query": format!("q{index}"),
                    "returned_matches": 1,
                    "results": [{"ref": format!("proto@0.0.1/action-{index}"), "kind":"action"}]
                })
                .to_string(),
            );
        }
        let projection = memory
            .tool_memory_projection(64)
            .expect("projection should exist");
        let estimated = projection
            .get("estimated_tokens")
            .and_then(Value::as_u64)
            .expect("estimated tokens");
        let budget = projection
            .get("token_budget")
            .and_then(Value::as_u64)
            .expect("budget");
        assert_eq!(budget, TOOL_MEMORY_MIN_TOKENS as u64);
        assert!(estimated <= budget);
    }

    #[test]
    fn tool_memory_projection_dedupes_catalog_and_detail_refs() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("s", "snap-3");
        memory.insert(
            "catalog.search:k1".to_string(),
            json!({
                "query":"transfer",
                "returned_matches":2,
                "results":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action"},
                    {"ref":"evm-native-utils@0.0.1/native-transfer","kind":"action"}
                ]
            })
            .to_string(),
        );
        memory.insert(
            "catalog.search:k2".to_string(),
            json!({
                "query":"transfer",
                "returned_matches":2,
                "results":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action"},
                    {"ref":"erc20@0.0.2/approve","kind":"action"}
                ]
            })
            .to_string(),
        );
        memory.insert(
            "get_candidate_detail:k3".to_string(),
            json!({
                "details":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true}]},
                    {"ref":"erc20@0.0.2/approve","kind":"action","params":[{"name":"spender","required":true}]}
                ]
            })
            .to_string(),
        );
        memory.insert(
            "get_candidate_detail:k4".to_string(),
            json!({
                "details":[
                    {"ref":"erc20@0.0.2/transfer","kind":"action","params":[{"name":"to","required":true}]}
                ]
            })
            .to_string(),
        );

        let projection = memory
            .tool_memory_projection(1200)
            .expect("projection should exist");
        let top_ref_a = projection
            .pointer("/recent/catalog_search/0/top_refs/0/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let top_ref_b = projection
            .pointer("/recent/catalog_search/0/top_refs/1/ref")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        assert_ne!(top_ref_a, top_ref_b);
        let signatures = projection
            .pointer("/recent/candidate_detail")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut refs = BTreeSet::<String>::new();
        for entry in signatures {
            for signature in entry
                .get("signatures")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(reference) = signature.get("ref").and_then(Value::as_str) {
                    refs.insert(reference.to_string());
                }
            }
        }
        assert!(refs.contains("erc20@0.0.2/transfer"));
        assert!(refs.contains("erc20@0.0.2/approve"));
    }

    #[test]
    fn tool_memory_projection_prioritizes_guide_schema_topic() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("s", "snap-4");
        memory.insert(
            "guide.get:k1".to_string(),
            json!({
                "kind":"topic",
                "topic":{"topic":"valueref","summary":"ValueRef forms"}
            })
            .to_string(),
        );
        memory.insert(
            "guide.get:k2".to_string(),
            json!({
                "kind":"schema",
                "schema":{"id":"ais-agent-intent/0.0.1","json":{"$defs":{"x":{}}}}
            })
            .to_string(),
        );
        memory.insert(
            "guide.get:k3".to_string(),
            json!({
                "kind":"schema",
                "schema":{"id":"ais-plan-sketch/0.1.0","json":{"$defs":{"segment":{},"step":{}}}}
            })
            .to_string(),
        );
        memory.insert(
            "guide.get:k4".to_string(),
            json!({
                "kind":"topic",
                "topic":{"topic":"cel","summary":"CEL guide"}
            })
            .to_string(),
        );
        // duplicate higher-priority id should be deduped
        memory.insert(
            "guide.get:k5".to_string(),
            json!({
                "kind":"schema",
                "schema":{"id":"ais-plan-sketch/0.1.0","json":{"$defs":{"segment":{}}}}
            })
            .to_string(),
        );

        let projection = memory
            .tool_memory_projection(1200)
            .expect("projection should exist");
        assert_eq!(
            projection
                .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/defs/0")
                .and_then(Value::as_str),
            Some("segment")
        );
        assert_eq!(
            projection
                .pointer("/recent/guide/topic/cel/summary")
                .and_then(Value::as_str),
            Some("CEL guide")
        );
        let schema_keys = projection
            .pointer("/recent/guide/schema")
            .and_then(Value::as_object)
            .map(|items| items.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        assert_eq!(
            schema_keys
                .iter()
                .filter(|id| id.as_str() == "ais-plan-sketch/0.1.0")
                .count(),
            1
        );
    }

    #[test]
    fn tool_memory_projection_guide_schema_prefers_full_over_digest() {
        let mut memory = PlanningMemory::default();
        memory.ensure_scope("s", "snap-5");
        memory.insert(
            "guide.get:k1".to_string(),
            json!({
                "kind":"schema",
                "schema":{
                    "id":"ais-plan-sketch/0.1.0",
                    "mode":"digest",
                    "digest":{"defs":["segment"]}
                }
            })
            .to_string(),
        );
        memory.insert(
            "guide.get:k2".to_string(),
            json!({
                "kind":"schema",
                "schema":{
                    "id":"ais-plan-sketch/0.1.0",
                    "mode":"full",
                    "json":{"$defs":{"segment":{},"step":{}}}
                }
            })
            .to_string(),
        );

        let projection = memory
            .tool_memory_projection(1200)
            .expect("projection should exist");
        assert_eq!(
            projection
                .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/mode")
                .and_then(Value::as_str),
            Some("full")
        );
        let defs = projection
            .pointer("/recent/guide/schema/ais-plan-sketch~10.1.0/defs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(defs.iter().any(|item| item.as_str() == Some("segment")));
        assert!(defs.iter().any(|item| item.as_str() == Some("step")));
    }
}
