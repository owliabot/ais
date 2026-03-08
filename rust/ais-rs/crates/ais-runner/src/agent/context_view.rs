use super::context::packing::PackPhaseHint;
use super::context::{budgeter, envelope, projector};
use super::input_store::InputStore;
use super::runtime_facts_store::RuntimeFactsStore;
use super::state_summary::StateSummary;
use ais_engine::EngineRunnerState;
use serde_json::Value;

pub(super) const DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET: usize =
    budgeter::DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET;

/// Result of building a new context summary.
pub(super) struct ContextSummaryResult {
    /// Budgeted + enveloped Value for LLM prompt construction.
    pub packed: Value,
    /// Pre-budget typed summary for internal typed field access.
    pub typed: StateSummary,
}

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

    #[cfg(test)]
    pub(super) fn next_summary(
        &mut self,
        state: &EngineRunnerState,
        completed_segments: usize,
        done: bool,
        previous_error: Option<&Value>,
        input_store: Option<&InputStore>,
        tool_memory_projection: Option<&Value>,
    ) -> Value {
        self.next_summary_result_with_runtime_facts(
            state,
            completed_segments,
            done,
            previous_error,
            input_store,
            None,
            tool_memory_projection,
        )
        .packed
    }

    pub(super) fn next_summary_result_with_runtime_facts(
        &mut self,
        state: &EngineRunnerState,
        completed_segments: usize,
        done: bool,
        previous_error: Option<&Value>,
        input_store: Option<&InputStore>,
        runtime_facts_store: Option<&RuntimeFactsStore>,
        tool_memory_projection: Option<&Value>,
    ) -> ContextSummaryResult {
        let typed = projector::build_projected_summary_base_with_runtime_facts(
            state,
            completed_segments,
            done,
            previous_error,
            input_store,
            runtime_facts_store,
            tool_memory_projection,
        );
        let payload = budgeter::budget_and_compact_summary(
            typed.to_value(),
            state,
            self.token_budget,
            PackPhaseHint::Default,
        );
        self.version = self.version.saturating_add(1);
        let env = envelope::ContextEnvelope::from_payload(
            &payload,
            self.version,
            self.last_hash.as_deref(),
        );
        self.last_hash = Some(env.hash.clone());
        let packed = env.to_compat_summary(payload.clone());
        ContextSummaryResult { packed, typed }
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
    build_projected_summary_with_runtime_facts_and_budget(
        state,
        completed_segments,
        done,
        previous_error,
        input_store,
        None,
        tool_memory_projection,
        DEFAULT_PLANNER_CONTEXT_TOKEN_BUDGET,
    )
}

fn build_projected_summary_with_runtime_facts_and_budget(
    state: &EngineRunnerState,
    completed_segments: usize,
    done: bool,
    previous_error: Option<&Value>,
    input_store: Option<&InputStore>,
    runtime_facts_store: Option<&RuntimeFactsStore>,
    tool_memory_projection: Option<&Value>,
    token_budget: usize,
) -> Value {
    let typed = projector::build_projected_summary_base_with_runtime_facts(
        state,
        completed_segments,
        done,
        previous_error,
        input_store,
        runtime_facts_store,
        tool_memory_projection,
    );
    budgeter::budget_and_compact_summary(
        typed.to_value(),
        state,
        token_budget,
        PackPhaseHint::Default,
    )
}

#[cfg(test)]
#[path = "tests/context_view_module.rs"]
mod tests;
