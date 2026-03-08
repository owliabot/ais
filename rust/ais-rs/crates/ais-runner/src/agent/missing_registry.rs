use super::input_normalize;
use super::ref_model::RefPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Typed missing registry entrypoint used by compile/precheck/recovery paths.
/// Wave-1 emits canonical refs (`inputs/facts/nodes`) while preserving typed `RefPath` items.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MissingItemSource {
    MissingRefsField,
    QuestionsField,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MissingItem {
    pub missing_ref: RefPath,
    pub source: MissingItemSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct CompileMissingInputCollection {
    pub missing_refs: Vec<String>,
    pub issues: Vec<Value>,
}

#[allow(dead_code)]
pub(super) fn collect_missing_items(payload: &Value) -> Vec<MissingItem> {
    let mut indexed = BTreeMap::<(String, MissingItemSource), MissingItem>::new();

    for raw in payload
        .get("missing_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        if let Some(reference) = raw.as_str() {
            collect_missing_items_from_raw(
                reference,
                Some(raw),
                MissingItemSource::MissingRefsField,
                &mut indexed,
            );
            continue;
        }
        let reference = raw
            .get("ref")
            .or_else(|| raw.get("missing_ref"))
            .or_else(|| raw.get("path"))
            .and_then(Value::as_str);
        if let Some(reference) = reference {
            collect_missing_items_from_raw(
                reference,
                Some(raw),
                MissingItemSource::MissingRefsField,
                &mut indexed,
            );
        }
    }

    for question in payload
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        let Some(reference) = question.get("id").and_then(Value::as_str) else {
            continue;
        };
        collect_missing_items_from_raw(
            reference,
            None,
            MissingItemSource::QuestionsField,
            &mut indexed,
        );
    }

    indexed.into_values().collect::<Vec<_>>()
}

pub(super) fn collect_todo_precheck_missing_refs<F>(
    required_facts: &[String],
    mut has_ref: F,
) -> Vec<String>
where
    F: FnMut(&str) -> bool,
{
    let mut refs = BTreeSet::<String>::new();
    for fact in required_facts {
        let Some(canonical_ref) = input_normalize::canonical_missing_ref(fact) else {
            continue;
        };
        if !has_ref(canonical_ref.as_str()) {
            refs.insert(canonical_ref);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn collect_compile_missing_input(
    error_payload: &Value,
) -> CompileMissingInputCollection {
    let issues = error_payload
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut missing_refs = BTreeSet::<String>::new();
    let mut missing_input_issues = Vec::<Value>::new();
    for issue in issues {
        let reference = issue.get("reference").and_then(Value::as_str).unwrap_or("");
        let kind = issue.get("kind").and_then(Value::as_str).unwrap_or("");
        if reference == "unknown_input_ref" {
            missing_input_issues.push(issue.clone());
            if let Some(suggested_ref) = issue.get("suggested_ref").and_then(Value::as_str) {
                collect_ref_from_raw(suggested_ref, Some(&issue), &mut missing_refs);
            }
            for candidate in issue
                .get("candidates")
                .and_then(Value::as_array)
                .into_iter()
                .flat_map(|items| items.iter())
                .filter_map(Value::as_str)
            {
                collect_ref_from_raw(candidate, Some(&issue), &mut missing_refs);
            }
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                collect_missing_refs_from_message(message, &mut missing_refs);
            }
            continue;
        }
        if is_write_gate_missing_input_issue(&issue, kind) {
            missing_input_issues.push(issue.clone());
            if let Some(required_fact) = issue.get("required_fact").and_then(Value::as_str) {
                collect_ref_from_raw(required_fact, Some(&issue), &mut missing_refs);
            }
            if let Some(message) = issue.get("message").and_then(Value::as_str) {
                collect_missing_refs_from_message(message, &mut missing_refs);
            }
            continue;
        }
    }

    if missing_input_issues.is_empty() {
        return CompileMissingInputCollection::default();
    }
    if let Some(message) = error_payload.get("message").and_then(Value::as_str) {
        collect_missing_refs_from_message(message, &mut missing_refs);
    }
    let filtered_missing_refs = missing_refs
        .into_iter()
        .filter(|reference| reference.starts_with("inputs."))
        .collect::<Vec<_>>();
    if filtered_missing_refs.is_empty() {
        return CompileMissingInputCollection::default();
    }

    CompileMissingInputCollection {
        missing_refs: filtered_missing_refs,
        issues: missing_input_issues,
    }
}

pub(super) fn collect_missing_refs_from_payload(payload: &Value) -> Vec<String> {
    let mut refs = BTreeSet::<String>::new();
    for raw in payload
        .get("missing_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
    {
        if let Some(reference) = raw.as_str() {
            collect_ref_from_raw(reference, Some(raw), &mut refs);
            continue;
        }
        let reference = raw
            .get("ref")
            .or_else(|| raw.get("missing_ref"))
            .or_else(|| raw.get("path"))
            .and_then(Value::as_str);
        if let Some(reference) = reference {
            collect_ref_from_raw(reference, Some(raw), &mut refs);
        }
    }
    refs.into_iter().collect::<Vec<_>>()
}

pub(super) fn collect_question_refs(questions: &[Value]) -> Vec<String> {
    let payload = serde_json::json!({
        "questions": questions,
    });
    collect_missing_items(&payload)
        .into_iter()
        .map(|item| item.missing_ref.as_canonical_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

pub(super) fn collect_missing_refs_from_message(
    message: &str,
    missing_refs: &mut BTreeSet<String>,
) {
    for (index, chunk) in message.split('`').enumerate() {
        if index % 2 == 1 {
            collect_ref_from_raw(chunk, None, missing_refs);
        }
    }
    if let Some(suffix) = message
        .split_once("suggested_ref=")
        .map(|(_, value)| value.trim())
    {
        let candidate = suffix
            .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | '('))
            .next()
            .unwrap_or_default();
        collect_ref_from_raw(candidate, None, missing_refs);
    }
}

pub(super) fn collect_ref_from_raw(
    raw: &str,
    metadata: Option<&Value>,
    refs: &mut BTreeSet<String>,
) {
    let Some(canonical_ref) = input_normalize::canonical_missing_ref(raw) else {
        return;
    };
    let Some(slot) = canonical_ref.strip_prefix("inputs.") else {
        refs.insert(canonical_ref);
        return;
    };
    for leaf in input_normalize::expand_missing_input_slot(slot, metadata) {
        refs.insert(format!("inputs.{leaf}"));
    }
}

fn is_write_gate_missing_input_issue(issue: &Value, kind: &str) -> bool {
    if kind != "write_gate_missing" {
        return false;
    }
    let reason_code = issue
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason_code == "missing_required_input" {
        return true;
    }
    if write_gate_issue_has_required_input_fact(issue) {
        return true;
    }
    let mut message_refs = BTreeSet::<String>::new();
    if let Some(message) = issue.get("message").and_then(Value::as_str) {
        collect_missing_refs_from_message(message, &mut message_refs);
    }
    !message_refs.is_empty() || reason_code == "missing_required_input"
}

fn write_gate_issue_has_required_input_fact(issue: &Value) -> bool {
    if issue
        .get("required_fact")
        .and_then(Value::as_str)
        .and_then(input_normalize::canonical_missing_ref)
        .is_some_and(|reference| reference.starts_with("inputs."))
    {
        return true;
    }
    issue
        .get("required_facts")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(Value::as_str)
        .filter_map(input_normalize::canonical_missing_ref)
        .any(|reference| reference.starts_with("inputs."))
}

fn collect_missing_items_from_raw(
    raw: &str,
    metadata: Option<&Value>,
    source: MissingItemSource,
    indexed: &mut BTreeMap<(String, MissingItemSource), MissingItem>,
) {
    let mut refs = BTreeSet::<String>::new();
    collect_ref_from_raw(raw, metadata, &mut refs);
    for canonical in refs {
        let Some(path) = RefPath::parse(canonical.as_str()) else {
            continue;
        };
        indexed.insert(
            (canonical, source.clone()),
            MissingItem {
                missing_ref: path,
                source: source.clone(),
            },
        );
    }
}

#[cfg(test)]
#[path = "tests/missing_registry.rs"]
mod tests;
