use super::super::orchestrator::SegmentedAgentContext;
use ais_engine::EngineRunnerState;
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn runtime_has_ref_typed(
    state_summary: Option<&super::super::state_summary::StateSummary>,
    reference: &str,
) -> bool {
    let Some(path) = super::super::input_normalize::canonical_missing_ref_path(reference) else {
        return false;
    };
    match path {
        super::super::ref_model::RefPath::Input { slot } => {
            typed_runtime_has_input_slot(state_summary, slot.as_str())
        }
        super::super::ref_model::RefPath::Fact { key } => {
            typed_runtime_has_fact_key(state_summary, key.as_str())
        }
        super::super::ref_model::RefPath::NodeOutput {
            step_id,
            field_path,
        } => {
            let expected = format!("nodes.{step_id}.outputs.{field_path}");
            state_summary
                .map(|summary| {
                    summary
                        .node_output_refs_known_refs()
                        .iter()
                        .any(|raw_ref| *raw_ref == expected)
                })
                .unwrap_or(false)
        }
    }
}

pub(crate) fn filter_unresolved_refs_typed(
    state_summary: Option<&super::super::state_summary::StateSummary>,
    references: &[String],
) -> Vec<String> {
    references
        .iter()
        .filter(|reference| !runtime_has_ref_typed(state_summary, reference))
        .cloned()
        .collect::<Vec<_>>()
}

#[allow(dead_code)]
pub(crate) fn runtime_has_ref(state_summary: Option<&Value>, reference: &str) -> bool {
    let Some(path) = super::super::input_normalize::canonical_missing_ref_path(reference) else {
        return false;
    };
    match path {
        super::super::ref_model::RefPath::Input { slot } => {
            runtime_has_input_slot(state_summary, slot.as_str())
        }
        super::super::ref_model::RefPath::Fact { key } => {
            runtime_has_fact_key(state_summary, key.as_str())
        }
        super::super::ref_model::RefPath::NodeOutput {
            step_id,
            field_path,
        } => {
            runtime_readable_node_output_value(state_summary, step_id.as_str(), field_path.as_str())
                .is_some()
        }
    }
}

fn value_at_dotted_path_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    dotted: &str,
) -> Option<&'a Value> {
    let mut segments = dotted.split('.').filter(|part| !part.is_empty());
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for segment in segments {
        current = current.get(segment)?;
    }
    Some(current)
}

pub(crate) fn runtime_has_input_ref(state_summary: Option<&Value>, input_ref: &str) -> bool {
    let Some(canonical_slot) = super::super::input_normalize::normalize_input_slot_key(input_ref)
    else {
        return false;
    };
    runtime_has_input_slot(state_summary, canonical_slot.as_str())
}

fn runtime_has_input_slot(state_summary: Option<&Value>, canonical_slot: &str) -> bool {
    let canonical_ref = format!("inputs.{canonical_slot}");
    if super::super::reference_inventory::ReferenceInventory::build(state_summary)
        .input_refs()
        .iter()
        .any(|known| known == &canonical_ref)
    {
        return true;
    }
    let has_input_meta = state_summary
        .and_then(|summary| summary.pointer("/input_store/meta"))
        .and_then(|meta| {
            meta.as_object()
                .and_then(|object| object.get(canonical_slot))
                .or_else(|| value_at_dotted_path(meta, canonical_slot))
        })
        .is_some();
    if has_input_meta
        && state_summary
            .and_then(|summary| summary.pointer("/input_store/facts"))
            .and_then(|facts| {
                facts
                    .as_object()
                    .and_then(|object| object.get(canonical_slot))
                    .or_else(|| value_at_dotted_path(facts, canonical_slot))
            })
            .is_some()
    {
        return true;
    }
    if state_summary
        .and_then(|summary| summary.pointer("/intent_slots/resolved_inputs"))
        .and_then(|inputs| {
            inputs
                .as_object()
                .and_then(|object| object.get(canonical_slot))
                .or_else(|| value_at_dotted_path(inputs, canonical_slot))
        })
        .is_some()
    {
        return true;
    }
    false
}

fn typed_runtime_has_input_slot(
    state_summary: Option<&super::super::state_summary::StateSummary>,
    canonical_slot: &str,
) -> bool {
    let canonical_ref = format!("inputs.{canonical_slot}");
    if super::super::known_input_refs_from_typed_summary(state_summary)
        .iter()
        .any(|known| known == &canonical_ref)
    {
        return true;
    }
    let has_input_meta = state_summary
        .and_then(|summary| {
            summary.input_store_meta().and_then(|meta| {
                meta.get(canonical_slot)
                    .or_else(|| value_at_dotted_path_object(meta, canonical_slot))
            })
        })
        .is_some();
    if has_input_meta
        && state_summary
            .and_then(|summary| summary.input_store_facts())
            .and_then(|facts| {
                facts
                    .get(canonical_slot)
                    .or_else(|| value_at_dotted_path_object(facts, canonical_slot))
            })
            .is_some()
    {
        return true;
    }
    if state_summary
        .and_then(|summary| summary.intent_slots_view().resolved_input(canonical_slot))
        .is_some()
    {
        return true;
    }
    false
}

fn runtime_has_fact_key(state_summary: Option<&Value>, key: &str) -> bool {
    let canonical = format!("facts.{key}");
    state_summary
        .and_then(|summary| summary.pointer("/runtime_facts/facts"))
        .and_then(|facts| {
            facts
                .as_object()
                .and_then(|object| object.get(canonical.as_str()))
                .or_else(|| value_at_dotted_path(facts, canonical.as_str()))
        })
        .is_some()
}

fn typed_runtime_has_fact_key(
    state_summary: Option<&super::super::state_summary::StateSummary>,
    key: &str,
) -> bool {
    let canonical = format!("facts.{key}");
    state_summary
        .and_then(|summary| summary.runtime_facts_view().fact(canonical.as_str()))
        .is_some()
}

fn input_store_meta_allows_slot(meta: Option<&serde_json::Map<String, Value>>, slot: &str) -> bool {
    let Some(meta) = meta else {
        return true;
    };
    if let Some(entry) = meta
        .get(slot)
        .or_else(|| value_at_dotted_path_object(meta, slot))
    {
        return meta_entry_has_any_source(entry);
    }
    let prefix = format!("{slot}.");
    let mut saw_descendant = false;
    let mut has_true_input_descendant = false;
    for (key, entry) in meta {
        if !key.starts_with(prefix.as_str()) {
            continue;
        }
        saw_descendant = true;
        if meta_entry_has_any_source(entry) {
            has_true_input_descendant = true;
            break;
        }
    }
    if !saw_descendant {
        return true;
    }
    has_true_input_descendant
}

fn meta_entry_has_any_source(entry: &Value) -> bool {
    if let Some(source) = entry.get("source").and_then(Value::as_str) {
        return !source.trim().is_empty();
    }
    entry
        .as_object()
        .is_none_or(|object| object.values().any(meta_entry_has_any_source))
}

#[allow(dead_code)]
fn runtime_readable_node_output_value<'a>(
    state_summary: Option<&'a Value>,
    step_id: &str,
    field_path: &str,
) -> Option<&'a Value> {
    let nodes = state_summary
        .and_then(|summary| summary.pointer("/nodes"))
        .and_then(Value::as_object)?;
    for (node_id, node_value) in nodes {
        if !runtime_node_id_matches_step(node_id.as_str(), step_id) {
            continue;
        }
        let Some(outputs) = node_value.get("outputs") else {
            continue;
        };
        let Some(value) = value_at_dotted_path(outputs, field_path) else {
            continue;
        };
        if runtime_value_readable(value) {
            return Some(value);
        }
    }
    None
}

#[allow(dead_code)]
fn runtime_node_id_matches_step(node_id: &str, step_id: &str) -> bool {
    let normalized_node = node_id.trim();
    let normalized_step = step_id.trim();
    if normalized_node.is_empty() || normalized_step.is_empty() {
        return false;
    }
    if normalized_node == normalized_step {
        return true;
    }
    if normalized_node
        .rsplit_once("__")
        .map(|(_, suffix)| suffix.trim())
        .is_some_and(|suffix| suffix == normalized_step)
    {
        return true;
    }
    normalized_node
        .rsplit_once('/')
        .map(|(_, suffix)| suffix.trim())
        .is_some_and(|suffix| suffix == normalized_step)
}

fn runtime_value_readable(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StaticRefillOutcome {
    pub(crate) resolved_refs: Vec<String>,
    pub(crate) ambiguous_bindings: Vec<AmbiguousBinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct AmbiguousBinding {
    pub(crate) missing_ref: String,
    pub(crate) candidate_refs: Vec<String>,
}

pub(crate) fn apply_static_missing_ref_refill(
    state: &mut EngineRunnerState,
    context: &mut SegmentedAgentContext,
    missing_refs: &[String],
    phase_hint: &str,
    scope_id: &str,
) -> StaticRefillOutcome {
    let summary_snapshot = context.packed_summary().clone();
    let mut resolved = BTreeSet::<String>::new();
    let mut ambiguous = Vec::<AmbiguousBinding>::new();
    for raw_ref in missing_refs {
        let Some(slot) = super::super::input_normalize::normalize_input_slot_key(raw_ref) else {
            continue;
        };
        let canonical_ref = format!("inputs.{slot}");
        if runtime_has_input_ref(summary_snapshot.as_ref(), canonical_ref.as_str()) {
            resolved.insert(canonical_ref);
            continue;
        }
        match resolve_static_input_binding(summary_snapshot.as_ref(), slot.as_str()) {
            StaticInputBindingDecision::Resolved(value) => {
                let provenance = format!("autofill.static.{phase_hint}.{scope_id}.{slot}");
                super::super::input_normalize::set_runtime_input_value(
                    &mut state.runtime,
                    slot.as_str(),
                    value.clone(),
                );
                let _ = super::super::upsert_store_value_with_source(
                    context.input_store_mut(),
                    slot.as_str(),
                    value,
                    super::super::input_store::InputValueLayer::Derived,
                    "autofill_static",
                    85,
                    provenance,
                );
                resolved.insert(canonical_ref);
            }
            StaticInputBindingDecision::Ambiguous(candidate_refs) => {
                ambiguous.push(AmbiguousBinding {
                    missing_ref: canonical_ref,
                    candidate_refs,
                });
            }
            StaticInputBindingDecision::Unresolved => {
                if let Some(value) =
                    resolve_from_completed_node_outputs(summary_snapshot.as_ref(), slot.as_str())
                {
                    let provenance = format!("autofill.node_output.{phase_hint}.{scope_id}.{slot}");
                    super::super::input_normalize::set_runtime_input_value(
                        &mut state.runtime,
                        slot.as_str(),
                        value.clone(),
                    );
                    let _ = super::super::upsert_store_value_with_source(
                        context.input_store_mut(),
                        slot.as_str(),
                        value,
                        super::super::input_store::InputValueLayer::Derived,
                        "autofill_node_output",
                        85,
                        provenance,
                    );
                    resolved.insert(canonical_ref);
                }
            }
        }
    }
    if !resolved.is_empty() {
        context.refresh_state_summary(state, false);
    }
    StaticRefillOutcome {
        resolved_refs: resolved.into_iter().collect::<Vec<_>>(),
        ambiguous_bindings: ambiguous,
    }
}

fn resolve_static_input_binding(
    state_summary: Option<&Value>,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(summary) = state_summary else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut candidates = vec![slot.to_string()];
    for alias in super::heuristics::static_input_alias_slots(slot) {
        if !candidates.contains(&alias) {
            candidates.push(alias);
        }
    }
    for candidate in candidates {
        if let Some(value) = summary
            .pointer("/input_store/facts")
            .filter(|_| {
                input_store_meta_allows_slot(
                    summary
                        .pointer("/input_store/meta")
                        .and_then(Value::as_object),
                    candidate.as_str(),
                )
            })
            .and_then(|facts| {
                facts
                    .as_object()
                    .and_then(|object| object.get(candidate.as_str()))
                    .or_else(|| value_at_dotted_path(facts, candidate.as_str()))
            })
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
        if let Some(value) = summary
            .pointer("/intent_slots/resolved_inputs")
            .and_then(Value::as_object)
            .and_then(|resolved_inputs| resolved_inputs.get(candidate.as_str()))
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
    }
    resolve_static_input_value_by_semantic_match(summary, slot)
}

fn resolve_static_input_binding_typed(
    state_summary: Option<&super::super::state_summary::StateSummary>,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(summary) = state_summary else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut candidates = vec![slot.to_string()];
    for alias in super::heuristics::static_input_alias_slots(slot) {
        if !candidates.contains(&alias) {
            candidates.push(alias);
        }
    }
    for candidate in candidates {
        if let Some(value) = summary
            .input_store_meta()
            .filter(|meta| input_store_meta_allows_slot(Some(*meta), candidate.as_str()))
            .and_then(|_| {
                summary.input_store_facts().and_then(|facts| {
                    facts
                        .get(candidate.as_str())
                        .or_else(|| value_at_dotted_path_object(facts, candidate.as_str()))
                })
            })
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
        if let Some(value) = summary
            .intent_slots_view()
            .resolved_input(candidate.as_str())
            .cloned()
            .map(|value| unwrap_input_value(&value))
        {
            return StaticInputBindingDecision::Resolved(value);
        }
    }
    resolve_static_input_value_by_semantic_match_typed(summary, slot)
}

pub(crate) fn resolve_static_input_value_for_slot(
    state_summary: Option<&Value>,
    slot: &str,
) -> Option<Value> {
    match resolve_static_input_binding(state_summary, slot) {
        StaticInputBindingDecision::Resolved(value) => Some(value),
        StaticInputBindingDecision::Ambiguous(_) | StaticInputBindingDecision::Unresolved => None,
    }
}

pub(crate) fn resolve_static_input_value_for_slot_typed(
    typed_summary: Option<&super::super::state_summary::StateSummary>,
    state_summary: Option<&Value>,
    slot: &str,
) -> Option<Value> {
    match resolve_static_input_binding_typed(typed_summary, slot) {
        StaticInputBindingDecision::Resolved(value) => Some(value),
        StaticInputBindingDecision::Ambiguous(_) | StaticInputBindingDecision::Unresolved => {
            resolve_static_input_value_for_slot(state_summary, slot)
        }
    }
}

enum StaticInputBindingDecision {
    Resolved(Value),
    Ambiguous(Vec<String>),
    Unresolved,
}

fn resolve_static_input_value_by_semantic_match(
    summary: &Value,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(requirement) = TypedBindingRequirement::from_slot(slot) else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut scored = typed_binding_candidates(summary)
        .into_iter()
        .filter_map(|candidate| {
            score_typed_binding_candidate(&requirement, &candidate).map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    if scored.is_empty() {
        return StaticInputBindingDecision::Unresolved;
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((best_score, best_candidate)) = scored.first() else {
        return StaticInputBindingDecision::Unresolved;
    };
    if *best_score < 50 {
        return StaticInputBindingDecision::Unresolved;
    }
    let second_score = scored.get(1).map(|item| item.0).unwrap_or_default();
    let confident = *best_score >= 180 || best_score.saturating_sub(second_score) >= 15;
    if confident {
        return StaticInputBindingDecision::Resolved(best_candidate.value.clone());
    }
    let candidate_refs = scored
        .iter()
        .take(3)
        .map(|(_, candidate)| canonicalize_binding_candidate_ref(candidate.key.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if candidate_refs.len() < 2 {
        return StaticInputBindingDecision::Unresolved;
    }
    StaticInputBindingDecision::Ambiguous(candidate_refs)
}

fn resolve_static_input_value_by_semantic_match_typed(
    summary: &super::super::state_summary::StateSummary,
    slot: &str,
) -> StaticInputBindingDecision {
    let Some(requirement) = TypedBindingRequirement::from_slot(slot) else {
        return StaticInputBindingDecision::Unresolved;
    };
    let mut scored = typed_binding_candidates_typed(summary)
        .into_iter()
        .filter_map(|candidate| {
            score_typed_binding_candidate(&requirement, &candidate).map(|score| (score, candidate))
        })
        .collect::<Vec<_>>();
    if scored.is_empty() {
        return StaticInputBindingDecision::Unresolved;
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((best_score, best_candidate)) = scored.first() else {
        return StaticInputBindingDecision::Unresolved;
    };
    if *best_score < 50 {
        return StaticInputBindingDecision::Unresolved;
    }
    let second_score = scored.get(1).map(|item| item.0).unwrap_or_default();
    let confident = *best_score >= 180 || best_score.saturating_sub(second_score) >= 15;
    if confident {
        return StaticInputBindingDecision::Resolved(best_candidate.value.clone());
    }
    let candidate_refs = scored
        .iter()
        .take(3)
        .map(|(_, candidate)| canonicalize_binding_candidate_ref(candidate.key.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if candidate_refs.len() < 2 {
        return StaticInputBindingDecision::Unresolved;
    }
    StaticInputBindingDecision::Ambiguous(candidate_refs)
}

fn canonicalize_binding_candidate_ref(raw_key: &str) -> String {
    if let Some(slot) = super::super::input_normalize::normalize_input_slot_key(raw_key) {
        return format!("inputs.{slot}");
    }
    raw_key.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingValueType {
    Address,
    Boolean,
    Numeric,
    Chain,
    Text,
    Unknown,
}

#[derive(Debug, Clone)]
struct TypedBindingRequirement {
    normalized_slot: String,
    tokens: Vec<String>,
    expected_type: BindingValueType,
}

impl TypedBindingRequirement {
    fn from_slot(slot: &str) -> Option<Self> {
        let tokens = super::heuristics::semantic_tokens(slot);
        if tokens.is_empty() {
            return None;
        }
        Some(Self {
            normalized_slot: super::heuristics::normalize_semantic_key(slot),
            expected_type: infer_slot_type(slot, tokens.as_slice()),
            tokens,
        })
    }
}

#[derive(Debug, Clone)]
struct TypedBindingCandidate {
    key: String,
    normalized_key: String,
    tokens: Vec<String>,
    value: Value,
    value_type: BindingValueType,
    source_priority: u16,
}

fn typed_binding_candidates(summary: &Value) -> Vec<TypedBindingCandidate> {
    let mut candidates = Vec::<TypedBindingCandidate>::new();
    if let Some(facts) = summary
        .pointer("/input_store/facts")
        .and_then(Value::as_object)
    {
        let meta_map = summary
            .pointer("/input_store/meta")
            .and_then(Value::as_object);
        for (key, value) in facts {
            if !input_store_meta_allows_slot(meta_map, key.as_str()) {
                continue;
            }
            if input_store_binding_candidate_is_derived(meta_map, key.as_str()) {
                continue;
            }
            let source_priority = meta_map
                .and_then(|meta| meta.get(key.as_str()))
                .and_then(|entry| entry.get("source_priority"))
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    append_runtime_fact_binding_candidates(
        &mut candidates,
        summary
            .pointer("/runtime_facts/facts")
            .and_then(Value::as_object),
        summary
            .pointer("/runtime_facts/meta")
            .and_then(Value::as_object),
    );
    if let Some(facts) = summary
        .pointer("/intent_slots/resolved_inputs")
        .and_then(Value::as_object)
    {
        for (key, value) in facts {
            let source_priority = value
                .get("confidence")
                .and_then(Value::as_u64)
                .map(|confidence| 50 + confidence.min(50))
                .unwrap_or(50)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    candidates
}

fn typed_binding_candidates_typed(
    summary: &super::super::state_summary::StateSummary,
) -> Vec<TypedBindingCandidate> {
    let mut candidates = Vec::<TypedBindingCandidate>::new();
    if let Some(facts) = summary.input_store_facts() {
        let meta_map = summary.input_store_meta();
        for (key, value) in facts {
            if !input_store_meta_allows_slot(meta_map, key.as_str()) {
                continue;
            }
            if input_store_binding_candidate_is_derived(meta_map, key.as_str()) {
                continue;
            }
            let source_priority = meta_map
                .and_then(|meta| meta.get(key.as_str()))
                .and_then(|entry| entry.get("source_priority"))
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    append_runtime_fact_binding_candidates(
        &mut candidates,
        summary.runtime_facts_facts(),
        summary.runtime_facts_meta(),
    );
    if let Some(facts) = summary.intent_slots_resolved_inputs() {
        for (key, value) in facts {
            let source_priority = value
                .get("confidence")
                .and_then(Value::as_u64)
                .map(|confidence| 50 + confidence.min(50))
                .unwrap_or(50)
                .min(u16::MAX as u64) as u16;
            push_typed_binding_candidate(
                &mut candidates,
                key.as_str(),
                unwrap_input_value(value),
                source_priority,
            );
        }
    }
    candidates
}

fn push_typed_binding_candidate(
    candidates: &mut Vec<TypedBindingCandidate>,
    key: &str,
    value: Value,
    source_priority: u16,
) {
    let tokens = super::heuristics::semantic_tokens(key);
    if tokens.is_empty() {
        return;
    }
    candidates.push(TypedBindingCandidate {
        key: key.to_string(),
        normalized_key: super::heuristics::normalize_semantic_key(key),
        tokens,
        value_type: infer_value_type(&value),
        value,
        source_priority,
    });
}

fn input_store_binding_candidate_is_derived(
    meta: Option<&serde_json::Map<String, Value>>,
    slot: &str,
) -> bool {
    let Some(entry) = meta.and_then(|entries| {
        entries
            .get(slot)
            .or_else(|| value_at_dotted_path_object(entries, slot))
    }) else {
        return false;
    };
    let is_projected_provenance = entry
        .get("provenance")
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("input_store.projected."));
    let is_synthetic_projected_source = entry.get("source").and_then(Value::as_str)
        == Some("derived")
        && entry.get("layer").and_then(Value::as_str) == Some("derived");
    is_projected_provenance || is_synthetic_projected_source
}

fn append_runtime_fact_binding_candidates(
    candidates: &mut Vec<TypedBindingCandidate>,
    facts: Option<&serde_json::Map<String, Value>>,
    meta: Option<&serde_json::Map<String, Value>>,
) {
    let Some(facts) = facts else {
        return;
    };
    for (key, value) in facts {
        let candidate_key = key.strip_prefix("facts.").unwrap_or(key.as_str());
        let source_priority = meta
            .and_then(|entries| entries.get(key.as_str()))
            .and_then(|entry| entry.get("source_priority"))
            .and_then(Value::as_u64)
            .unwrap_or(55)
            .min(u16::MAX as u64) as u16;
        push_typed_binding_candidate(
            candidates,
            candidate_key,
            unwrap_input_value(value),
            source_priority,
        );
    }
}

fn score_typed_binding_candidate(
    requirement: &TypedBindingRequirement,
    candidate: &TypedBindingCandidate,
) -> Option<u16> {
    if !binding_type_compatible(requirement.expected_type, candidate.value_type) {
        return None;
    }
    if requirement.normalized_slot == candidate.normalized_key {
        return Some(220 + candidate.source_priority / 5);
    }

    let overlap = semantic_overlap(requirement.tokens.as_slice(), candidate.tokens.as_slice());
    if overlap.shared_total == 0 {
        return None;
    }

    let mut score = 0u16;
    score = score.saturating_add((overlap.shared_non_generic as u16).saturating_mul(35));
    score = score.saturating_add((overlap.shared_total as u16).saturating_mul(8));
    if requirement.expected_type == candidate.value_type {
        score = score.saturating_add(25);
    }
    score = score.saturating_add(candidate.source_priority.min(100) / 4);
    if overlap.slot_has_address && overlap.candidate_has_address {
        score = score.saturating_add(20);
    }
    if overlap.slot_has_decimals && overlap.candidate_has_decimals {
        score = score.saturating_add(20);
    }
    if candidate.key.starts_with(requirement.tokens[0].as_str()) {
        score = score.saturating_add(10);
    }
    Some(score)
}

#[derive(Default)]
struct SemanticOverlap {
    shared_total: usize,
    shared_non_generic: usize,
    slot_has_address: bool,
    candidate_has_address: bool,
    slot_has_decimals: bool,
    candidate_has_decimals: bool,
}

fn semantic_overlap(slot_tokens: &[String], candidate_tokens: &[String]) -> SemanticOverlap {
    let mut overlap = SemanticOverlap::default();
    overlap.slot_has_address = slot_tokens.iter().any(|token| token == "address");
    overlap.candidate_has_address = candidate_tokens.iter().any(|token| token == "address");
    overlap.slot_has_decimals = slot_tokens.iter().any(|token| token == "decimals");
    overlap.candidate_has_decimals = candidate_tokens.iter().any(|token| token == "decimals");

    let candidate_set = candidate_tokens
        .iter()
        .map(|token| token.as_str())
        .collect::<BTreeSet<_>>();
    for token in slot_tokens {
        if !candidate_set.contains(token.as_str()) {
            continue;
        }
        overlap.shared_total = overlap.shared_total.saturating_add(1);
        if !super::heuristics::is_generic_semantic_token(token.as_str()) {
            overlap.shared_non_generic = overlap.shared_non_generic.saturating_add(1);
        }
    }
    overlap
}

fn binding_type_compatible(expected: BindingValueType, actual: BindingValueType) -> bool {
    expected == BindingValueType::Unknown
        || actual == BindingValueType::Unknown
        || expected == actual
        || (expected == BindingValueType::Numeric && actual == BindingValueType::Text)
        || (expected == BindingValueType::Boolean && actual == BindingValueType::Text)
}

fn infer_slot_type(slot: &str, tokens: &[String]) -> BindingValueType {
    if is_address_like_slot(slot)
        || super::heuristics::semantic_has_any(
            tokens,
            &[
                "address",
                "owner",
                "recipient",
                "wallet",
                "account",
                "signer",
            ],
        )
    {
        return BindingValueType::Address;
    }
    if super::heuristics::semantic_has_any(
        tokens,
        &[
            "amount",
            "threshold",
            "decimals",
            "bps",
            "nonce",
            "limit",
            "deadline",
            "gas",
            "fee",
            "price",
        ],
    ) {
        return BindingValueType::Numeric;
    }
    if super::heuristics::semantic_has_any(tokens, &["chain", "chainid", "chainref"]) {
        return BindingValueType::Chain;
    }
    if super::heuristics::semantic_has_any(
        tokens,
        &[
            "bool", "enabled", "enable", "disabled", "allow", "should", "is", "has", "use",
        ],
    ) {
        return BindingValueType::Boolean;
    }
    BindingValueType::Unknown
}

fn infer_value_type(value: &Value) -> BindingValueType {
    match value {
        Value::Bool(_) => BindingValueType::Boolean,
        Value::Number(_) => BindingValueType::Numeric,
        Value::String(text) => {
            let trimmed = text.trim();
            if super::heuristics::is_evm_address(trimmed) {
                return BindingValueType::Address;
            }
            if trimmed.starts_with("eip155:") {
                return BindingValueType::Chain;
            }
            if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
                return BindingValueType::Boolean;
            }
            if trimmed.parse::<f64>().is_ok() {
                return BindingValueType::Numeric;
            }
            BindingValueType::Text
        }
        _ => BindingValueType::Unknown,
    }
}

fn is_address_like_slot(slot: &str) -> bool {
    slot.ends_with(".address") || slot.ends_with("_address")
}

fn unwrap_input_value(value: &Value) -> Value {
    value
        .as_object()
        .and_then(|object| object.get("value"))
        .cloned()
        .unwrap_or_else(|| value.clone())
}

fn value_at_dotted_path<'a>(root: &'a Value, dotted: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in dotted.split('.').filter(|part| !part.is_empty()) {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Resolve a missing `inputs.*` slot from completed node outputs by semantic matching.
/// E.g., `inputs.erc20_balance` can be matched from `nodes.seg_1__q_erc20_balance.outputs.balance`.
fn resolve_from_completed_node_outputs(state_summary: Option<&Value>, slot: &str) -> Option<Value> {
    let nodes = state_summary?
        .pointer("/nodes")
        .and_then(Value::as_object)?;
    let slot_tokens = super::heuristics::semantic_tokens(slot);
    if slot_tokens.is_empty() {
        return None;
    }
    let mut best_score = 0u16;
    let mut best_value = None::<Value>;
    for (node_id, node_value) in nodes {
        let Some(outputs) = node_value.get("outputs").and_then(Value::as_object) else {
            continue;
        };
        let step_id = node_id
            .rsplit_once('/')
            .map(|(_, suffix)| suffix)
            .or_else(|| node_id.rsplit_once("__").map(|(_, suffix)| suffix))
            .unwrap_or(node_id.as_str());
        let step_tokens = super::heuristics::semantic_tokens(step_id);
        for (field_name, field_value) in outputs {
            if !runtime_value_readable(field_value) {
                continue;
            }
            let mut field_tokens = step_tokens.clone();
            for token in super::heuristics::semantic_tokens(field_name) {
                if !field_tokens.contains(&token) {
                    field_tokens.push(token);
                }
            }
            let mut score = 0u16;
            let mut shared = 0u16;
            for token in &slot_tokens {
                if field_tokens.iter().any(|ft| ft == token) {
                    shared += 1;
                    if !super::heuristics::is_generic_semantic_token(token) {
                        score = score.saturating_add(40);
                    } else {
                        score = score.saturating_add(8);
                    }
                }
            }
            if shared == 0 {
                continue;
            }
            let clean_step = step_id
                .strip_prefix("q_")
                .or_else(|| step_id.strip_prefix("query_"))
                .unwrap_or(step_id);
            if slot == clean_step
                || slot == field_name
                || slot == format!("{clean_step}_{field_name}")
            {
                score = score.saturating_add(100);
            }
            if score > best_score {
                best_score = score;
                best_value = Some(field_value.clone());
            }
        }
    }
    if best_score >= 40 {
        best_value
    } else {
        None
    }
}
