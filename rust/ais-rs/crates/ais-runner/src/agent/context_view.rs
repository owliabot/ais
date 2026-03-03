use super::context::{budgeter, envelope, projector, prompt_compact};
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
        inject_prompt_compact_view(&mut summary);
        summary
    }
}

fn inject_prompt_compact_view(summary: &mut Value) {
    let compact = prompt_compact::build_prompt_compact(summary);
    if let Some(root) = summary.as_object_mut() {
        root.insert("prompt_compact".to_string(), compact);
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

#[cfg(test)]
#[path = "tests/context_view_module.rs"]
mod tests;
