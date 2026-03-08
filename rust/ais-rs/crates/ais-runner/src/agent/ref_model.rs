#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

const NON_BINDABLE_ROOTS: [&str; 14] = [
    "agent",
    "calculated",
    "contracts",
    "ctx",
    "params",
    "policy",
    "query",
    "runtime",
    "session",
    "state",
    "todo",
    "workspace",
    "input",
    "inputs",
];

/// Unified canonical reference model used by missing-recovery/fact binding flows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RefPath {
    Input { slot: String },
    Fact { key: String },
    NodeOutput { step_id: String, field_path: String },
}

impl RefPath {
    /// Parse a runtime/planner reference while preserving source namespace semantics.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let normalized = normalized_source_ref(raw)?;
        if let Some(parsed) = parse_node_output_ref(normalized.as_str()) {
            return Some(parsed);
        }
        if let Some(parsed) = parse_fact_ref(normalized.as_str()) {
            return Some(parsed);
        }
        parse_input_ref(normalized.as_str())
    }

    pub(crate) fn as_canonical_str(&self) -> String {
        match self {
            Self::Input { slot } => format!("inputs.{slot}"),
            Self::Fact { key } => format!("facts.{key}"),
            Self::NodeOutput {
                step_id,
                field_path,
            } => format!("nodes.{step_id}.outputs.{field_path}"),
        }
    }
}

impl Display for RefPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_canonical_str())
    }
}

fn normalized_source_ref(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '`' | '"' | '\'' | ',' | ';' | ')' | '(')
    });
    if trimmed.is_empty() {
        return None;
    }
    let value_side = trimmed
        .rsplit_once('=')
        .map(|(_, value)| value)
        .unwrap_or(trimmed)
        .trim();
    let value_side = value_side
        .strip_prefix("runtime.")
        .unwrap_or(value_side)
        .trim();
    let value_side = value_side
        .strip_suffix(".value")
        .unwrap_or(value_side)
        .trim();
    let value_side = value_side.trim_matches('.');
    if value_side.is_empty() {
        return None;
    }
    Some(value_side.to_string())
}

fn parse_input_ref(reference: &str) -> Option<RefPath> {
    let slot = reference
        .strip_prefix("inputs.")
        .or_else(|| reference.strip_prefix("input."))
        .unwrap_or(reference);
    let slot = normalize_input_slot(slot)?;
    Some(RefPath::Input { slot })
}

fn parse_fact_ref(reference: &str) -> Option<RefPath> {
    let key = if let Some(raw) = reference.strip_prefix("facts.") {
        raw
    } else if let Some(raw) = reference.strip_prefix("fact.") {
        raw
    } else if let Some(raw) = reference.strip_prefix("fact:") {
        raw
    } else {
        return None;
    };
    let key = normalize_fact_key(key)?;
    Some(RefPath::Fact { key })
}

fn parse_node_output_ref(reference: &str) -> Option<RefPath> {
    if let Some(rest) = reference.strip_prefix("nodes.") {
        return parse_node_output_dot_notation(rest);
    }
    if let Some(rest) = reference.strip_prefix("nodes[\"") {
        return parse_node_output_bracket_notation(rest, "\"");
    }
    if let Some(rest) = reference.strip_prefix("nodes['") {
        return parse_node_output_bracket_notation(rest, "'");
    }
    None
}

fn parse_node_output_dot_notation(rest: &str) -> Option<RefPath> {
    let (step_id_raw, field_path_raw) =
        if let Some((step_id, field_path)) = rest.split_once(".outputs.") {
            (step_id, field_path)
        } else if let Some((step_id, field_path)) = rest.split_once(".output.") {
            (step_id, field_path)
        } else {
            return None;
        };
    build_node_output_ref(step_id_raw, field_path_raw)
}

fn parse_node_output_bracket_notation(rest: &str, quote: &str) -> Option<RefPath> {
    let closing = format!("{quote}]");
    let (step_id_raw, tail) = rest.split_once(closing.as_str())?;
    let tail = tail.trim();
    if let Some(field_path_raw) = tail.strip_prefix(".outputs.") {
        return build_node_output_ref(step_id_raw, field_path_raw);
    }
    if let Some(field_path_raw) = tail.strip_prefix(".output.") {
        return build_node_output_ref(step_id_raw, field_path_raw);
    }
    None
}

fn build_node_output_ref(step_id_raw: &str, field_path_raw: &str) -> Option<RefPath> {
    let step_id = normalize_step_id(step_id_raw)?;
    let field_path = normalize_field_path(field_path_raw)?;
    Some(RefPath::NodeOutput {
        step_id,
        field_path,
    })
}

fn normalize_input_slot(raw: &str) -> Option<String> {
    let slot = normalize_dotted_identifier(raw)?;
    let root = slot
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if NON_BINDABLE_ROOTS.contains(&root.as_str()) {
        return None;
    }
    Some(slot)
}

fn normalize_fact_key(raw: &str) -> Option<String> {
    normalize_dotted_identifier(raw)
}

fn normalize_step_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_field_path(raw: &str) -> Option<String> {
    normalize_dotted_identifier(raw)
}

fn normalize_dotted_identifier(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('.');
    if trimmed.is_empty() {
        return None;
    }
    let segments = trimmed
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }
    if !segments.iter().all(|segment| {
        segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    }) {
        return None;
    }
    Some(segments.join("."))
}

#[cfg(test)]
#[path = "tests/ref_model.rs"]
mod tests;
