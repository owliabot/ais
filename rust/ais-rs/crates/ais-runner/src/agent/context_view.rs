use super::context::{budgeter, envelope, projector};
use super::input_store::InputStore;
use ais_engine::EngineRunnerState;
use serde_json::Value;

pub(super) const DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET: usize =
    budgeter::DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET;

#[derive(Debug, Clone)]
pub(super) struct PlanningContextManager {
    last_hash: Option<String>,
    version: u64,
    token_budget: usize,
}

impl PlanningContextManager {
    pub(super) fn with_token_budget(token_budget: usize) -> Self {
        Self {
            last_hash: None,
            version: 0,
            token_budget: token_budget.max(1),
        }
    }

    pub(super) fn next_summary(
        &mut self,
        state: &EngineRunnerState,
        completed_segments: usize,
        done: bool,
        previous_error: Option<&Value>,
        input_store: Option<&InputStore>,
        tool_memory_projection: Option<&Value>,
    ) -> Value {
        let payload = build_projected_summary_with_budget(
            state,
            completed_segments,
            done,
            previous_error,
            input_store,
            tool_memory_projection,
            self.token_budget,
        );
        self.version = self.version.saturating_add(1);
        let envelope = envelope::ContextEnvelope::from_payload(
            &payload,
            self.version,
            self.last_hash.as_deref(),
        );
        self.last_hash = Some(envelope.hash.clone());
        let mut summary = envelope.to_compat_summary(payload.clone());
        annotate_emitted_budget_metadata(&payload, &mut summary);
        summary
    }
}

impl Default for PlanningContextManager {
    fn default() -> Self {
        Self::with_token_budget(DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET)
    }
}

#[allow(dead_code)]
pub(super) fn build_projected_summary(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
    tool_memory_projection: Option<&Value>,
) -> Value {
    build_projected_summary_with_budget(
        state,
        completed_segments,
        done,
        previous_error,
        input_store,
        tool_memory_projection,
        DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET,
    )
}

fn build_projected_summary_with_budget(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
    tool_memory_projection: Option<&Value>,
    token_budget: usize,
) -> Value {
    let base = projector::build_projected_summary_base(
        state,
        completed_segments,
        done,
        previous_error,
        input_store,
        tool_memory_projection,
    );
    budgeter::budget_and_compact_summary(base, state, token_budget)
}

fn annotate_emitted_budget_metadata(payload: &Value, summary: &mut Value) {
    let payload_tokens = budgeter::estimate_tokens_json(payload);
    let token_limit = summary
        .pointer("/context_budget/token_limit")
        .and_then(Value::as_u64);
    let payload_core_tokens = summary
        .pointer("/context_budget/estimated_payload_core_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            summary
                .pointer("/context_budget/estimated_tokens")
                .and_then(Value::as_u64)
        })
        .unwrap_or(payload_tokens);
    let payload_metadata_tokens = payload_tokens.saturating_sub(payload_core_tokens);

    if let Some(context_budget) = summary
        .pointer_mut("/context_budget")
        .and_then(Value::as_object_mut)
    {
        context_budget.insert(
            "token_limit_scope".to_string(),
            Value::String("payload_core".to_string()),
        );
        context_budget.insert(
            "estimated_payload_tokens".to_string(),
            Value::Number(payload_tokens.into()),
        );
        context_budget.insert(
            "estimated_payload_metadata_tokens".to_string(),
            Value::Number(payload_metadata_tokens.into()),
        );
        context_budget.insert(
            "payload_within_token_limit".to_string(),
            Value::Bool(token_limit.is_some_and(|limit| payload_tokens <= limit)),
        );
    }

    let mut emitted_tokens = budgeter::estimate_tokens_json(summary);
    for _ in 0..3 {
        let emitted_metadata_tokens = emitted_tokens.saturating_sub(payload_tokens);
        if let Some(context_budget) = summary
            .pointer_mut("/context_budget")
            .and_then(Value::as_object_mut)
        {
            context_budget.insert(
                "estimated_emitted_tokens".to_string(),
                Value::Number(emitted_tokens.into()),
            );
            context_budget.insert(
                "estimated_emitted_metadata_tokens".to_string(),
                Value::Number(emitted_metadata_tokens.into()),
            );
            context_budget.insert(
                "emitted_within_token_limit".to_string(),
                Value::Bool(token_limit.is_some_and(|limit| emitted_tokens <= limit)),
            );
        }
        let refreshed = budgeter::estimate_tokens_json(summary);
        if refreshed == emitted_tokens {
            break;
        }
        emitted_tokens = refreshed;
    }
}

#[cfg(test)]
#[path = "tests/context_view_module.rs"]
mod tests;
