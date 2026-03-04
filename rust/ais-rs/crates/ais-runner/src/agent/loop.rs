use crate::error::RunnerError;
use ais_engine::{
    run_plan_once, EngineCommand, EngineCommandEnvelope, EngineCommandType, EngineEventRecord,
    EngineRunStatus, EngineRunnerOptions, EngineRunnerState,
};
use ais_sdk::PlanDocument;
use serde_json::{Map, Value};

use super::brain::DecisionPolicy;
use super::summary::{summarize_pause_with_context, PauseKind, PauseSummary};

#[derive(Debug, Clone, Copy)]
pub struct AgentLoopConfig {
    pub max_iterations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLoopResult {
    pub status: EngineRunStatus,
    pub iterations: usize,
}

#[derive(Debug, Clone)]
pub struct CommandBuilder {
    prefix: String,
    next_index: u64,
}

impl CommandBuilder {
    pub fn new(run_id: &str) -> Self {
        Self {
            prefix: format!("{run_id}-cmd"),
            next_index: 0,
        }
    }

    pub fn set_next_index_from_seen_ids(&mut self, seen_command_ids: &[String]) -> u64 {
        let mut max_seen = 0u64;
        for command_id in seen_command_ids {
            let Some(suffix) = command_id.strip_prefix(self.prefix.as_str()) else {
                continue;
            };
            let Some(index_text) = suffix.strip_prefix('-') else {
                continue;
            };
            let Ok(index) = index_text.parse::<u64>() else {
                continue;
            };
            max_seen = max_seen.max(index);
        }
        self.next_index = self.next_index.max(max_seen);
        max_seen
    }

    pub fn user_confirm(&mut self, node_id: &str, decision: &str) -> EngineCommandEnvelope {
        let mut data = Map::new();
        data.insert("node_id".to_string(), Value::String(node_id.to_string()));
        data.insert("decision".to_string(), Value::String(decision.to_string()));
        self.envelope(EngineCommandType::UserConfirm, data)
    }

    pub fn user_input(&mut self, input_id: &str, value: Value) -> EngineCommandEnvelope {
        let mut data = Map::new();
        data.insert("input_id".to_string(), Value::String(input_id.to_string()));
        data.insert("value".to_string(), value);
        self.envelope(EngineCommandType::UserInput, data)
    }

    pub fn user_select(
        &mut self,
        input_id: &str,
        selected_index: u64,
        options: Vec<Value>,
    ) -> EngineCommandEnvelope {
        let mut data = Map::new();
        data.insert("input_id".to_string(), Value::String(input_id.to_string()));
        data.insert(
            "selected_index".to_string(),
            Value::Number(selected_index.into()),
        );
        data.insert("options".to_string(), Value::Array(options));
        self.envelope(EngineCommandType::UserSelect, data)
    }

    pub fn cancel(&mut self) -> EngineCommandEnvelope {
        self.envelope(EngineCommandType::Cancel, Map::new())
    }

    pub(crate) fn envelope(
        &mut self,
        command_type: EngineCommandType,
        data: Map<String, Value>,
    ) -> EngineCommandEnvelope {
        let id = self.next_id();
        EngineCommandEnvelope::new(EngineCommand {
            id,
            command_type,
            data,
        })
    }

    fn next_id(&mut self) -> String {
        self.next_index = self.next_index.saturating_add(1);
        format!("{}-{:06}", self.prefix, self.next_index)
    }
}

pub fn run_agent_loop<P, F>(
    run_id: &str,
    plan: &PlanDocument,
    state: &mut EngineRunnerState,
    router: &ais_engine::RouterExecutor,
    solver: &dyn ais_engine::Solver,
    engine_options: &EngineRunnerOptions,
    loop_config: &AgentLoopConfig,
    builder: &mut CommandBuilder,
    decision_policy: &mut P,
    mut on_events: F,
) -> Result<AgentLoopResult, RunnerError>
where
    P: DecisionPolicy + ?Sized,
    F: FnMut(&EngineRunnerState, &[EngineEventRecord]) -> Result<(), RunnerError>,
{
    let mut iterations = 0usize;
    let mut pending_commands = Vec::<EngineCommandEnvelope>::new();

    loop {
        iterations += 1;
        if iterations > loop_config.max_iterations {
            return Err(RunnerError::IterationLimitExceeded(
                loop_config.max_iterations,
            ));
        }

        let result = run_plan_once(
            run_id,
            plan,
            state,
            router,
            solver,
            pending_commands.as_slice(),
            engine_options,
        );
        pending_commands.clear();

        on_events(state, result.events.as_slice())?;

        match result.status {
            EngineRunStatus::Completed | EngineRunStatus::Stopped => {
                return Ok(AgentLoopResult {
                    status: result.status,
                    iterations,
                });
            }
            EngineRunStatus::Paused => {
                if state.paused_reason.is_none() {
                    continue;
                }
                let pause_summary: PauseSummary = summarize_pause_with_context(
                    state.paused_reason.as_deref(),
                    result.events.as_slice(),
                    Some(plan),
                    Some(state),
                );
                if pause_summary.kind != PauseKind::NeedUserConfirm {
                    return Ok(AgentLoopResult {
                        status: EngineRunStatus::Paused,
                        iterations,
                    });
                }
                let next_commands = decision_policy.decide(&pause_summary, builder)?;
                if next_commands.is_empty() {
                    return Err(RunnerError::EventsIo(
                        "decision policy returned empty commands while paused".to_string(),
                    ));
                }
                pending_commands = next_commands;
            }
        }
    }
}
