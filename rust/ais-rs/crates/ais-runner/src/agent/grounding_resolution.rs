use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct GroundingCandidate {
    pub(crate) ready_hint: bool,
    pub(crate) missing_refs: Vec<String>,
    pub(crate) question_refs: Vec<String>,
    pub(crate) questions: Vec<Value>,
    pub(crate) resolved_inputs: BTreeMap<String, Value>,
    pub(crate) intent_facts: BTreeMap<String, Value>,
    pub(crate) confidence: BTreeMap<String, u8>,
    pub(crate) issues: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroundingResolutionState {
    Ready,
    NeedsUserInput,
}

#[derive(Debug, Clone)]
pub(crate) struct GroundingResolution {
    pub(crate) state: GroundingResolutionState,
    pub(crate) effective_missing_refs: Vec<String>,
    pub(crate) effective_questions: Vec<Value>,
    pub(crate) host_recovery_satisfied: bool,
    pub(crate) user_input_required: bool,
    pub(crate) planner_ready_hint: bool,
    pub(crate) ready_for_todos: bool,
}

pub(crate) fn normalize_grounding_candidate(
    ready_hint: bool,
    missing_refs: &[String],
    questions: &[Value],
    resolved_inputs: &BTreeMap<String, Value>,
    intent_facts: &BTreeMap<String, Value>,
    confidence: &BTreeMap<String, u8>,
    issues: &[Value],
) -> GroundingCandidate {
    let normalized_missing_refs = missing_refs
        .iter()
        .filter_map(|reference| super::input_normalize::canonical_missing_ref(reference))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let normalized_questions = questions.to_vec();
    let question_refs =
        super::missing_registry::collect_question_refs(normalized_questions.as_slice())
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    GroundingCandidate {
        ready_hint,
        missing_refs: normalized_missing_refs,
        question_refs,
        questions: normalized_questions,
        resolved_inputs: resolved_inputs.clone(),
        intent_facts: intent_facts.clone(),
        confidence: confidence.clone(),
        issues: issues.to_vec(),
    }
}

pub(crate) fn reconcile_grounding_candidate(
    typed_summary: Option<&super::state_summary::StateSummary>,
    candidate: &GroundingCandidate,
) -> GroundingResolution {
    let effective_missing_refs = effective_grounding_missing_refs(typed_summary, candidate);
    let effective_questions = build_grounding_user_questions(
        candidate.questions.as_slice(),
        effective_missing_refs.as_slice(),
    );
    let user_input_required = !effective_questions.is_empty() || !effective_missing_refs.is_empty();
    let had_actionable_follow_up = !candidate.questions.is_empty()
        || !candidate.missing_refs.is_empty()
        || !candidate.question_refs.is_empty();
    let host_recovery_satisfied = had_actionable_follow_up && !user_input_required;
    let has_host_grounding_signal = !candidate.resolved_inputs.is_empty()
        || !candidate.intent_facts.is_empty()
        || host_recovery_satisfied;
    let ready_for_todos = !user_input_required && has_host_grounding_signal;
    let state = if user_input_required {
        GroundingResolutionState::NeedsUserInput
    } else {
        GroundingResolutionState::Ready
    };
    GroundingResolution {
        state,
        effective_missing_refs,
        effective_questions,
        host_recovery_satisfied,
        user_input_required,
        planner_ready_hint: candidate.ready_hint,
        ready_for_todos,
    }
}

pub(crate) fn effective_grounding_missing_refs(
    typed_summary: Option<&super::state_summary::StateSummary>,
    candidate: &GroundingCandidate,
) -> Vec<String> {
    let refs = candidate
        .missing_refs
        .iter()
        .chain(candidate.question_refs.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    super::missing_resolution::filter_unresolved_refs_typed(typed_summary, refs.as_slice())
}

pub(crate) fn build_grounding_user_questions(
    questions: &[Value],
    effective_missing_refs: &[String],
) -> Vec<Value> {
    if !questions.is_empty() {
        let unresolved = effective_missing_refs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut covered = BTreeSet::<String>::new();
        let mut filtered = Vec::<Value>::new();
        for question in questions {
            let canonical_ref = question
                .get("id")
                .and_then(Value::as_str)
                .and_then(super::input_normalize::canonical_missing_ref);
            match canonical_ref {
                Some(reference) if unresolved.contains(reference.as_str()) => {
                    covered.insert(reference);
                    filtered.push(question.clone());
                }
                Some(_) => {}
                None => filtered.push(question.clone()),
            }
        }
        for reference in effective_missing_refs {
            if covered.contains(reference) {
                continue;
            }
            filtered.push(serde_json::json!({
                "id": reference,
                "question": format!("Provide a value for {reference}."),
                "required": true,
            }));
        }
        return filtered;
    }
    effective_missing_refs
        .iter()
        .map(|reference| {
            serde_json::json!({
                "id": reference,
                "question": format!("Provide a value for {reference}."),
                "required": true,
            })
        })
        .collect::<Vec<_>>()
}
