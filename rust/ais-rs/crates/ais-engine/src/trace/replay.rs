use crate::checkpoint::CheckpointDocument;
use crate::engine::{run_plan_once, EngineRunStatus, EngineRunnerOptions, EngineRunnerState};
use crate::events::{parse_event_jsonl_line, EngineEventRecord};
use crate::executor::RouterExecutor;
use crate::solver::Solver;
use ais_sdk::PlanDocument;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStatus {
    Completed,
    Paused,
    ReachedUntilNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayOptions {
    #[serde(default)]
    pub until_node: Option<String>,
    pub max_steps: usize,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            until_node: None,
            max_steps: 128,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayResult {
    pub status: ReplayStatus,
    pub events: Vec<EngineEventRecord>,
    #[serde(default)]
    pub completed_node_ids: Vec<String>,
    #[serde(default)]
    pub paused_reason: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("trace jsonl parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn replay_trace_events(events: &[EngineEventRecord], options: &ReplayOptions) -> ReplayResult {
    let mut out = Vec::<EngineEventRecord>::new();
    let mut completed = BTreeSet::<String>::new();

    for event in events {
        out.push(event.clone());
        if event.event.event_type == crate::events::EngineEventType::TxConfirmed {
            if let Some(node_id) = event.event.node_id.as_ref() {
                completed.insert(node_id.clone());
            }
        }
        if let Some(until_node) = options.until_node.as_deref() {
            if event.event.node_id.as_deref() == Some(until_node) {
                return ReplayResult {
                    status: ReplayStatus::ReachedUntilNode,
                    events: out,
                    completed_node_ids: completed.into_iter().collect(),
                    paused_reason: None,
                };
            }
        }
    }

    ReplayResult {
        status: ReplayStatus::Completed,
        events: out,
        completed_node_ids: completed.into_iter().collect(),
        paused_reason: None,
    }
}

pub fn replay_trace_jsonl(input: &str, options: &ReplayOptions) -> Result<ReplayResult, ReplayError> {
    let mut events = Vec::<EngineEventRecord>::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            continue;
        }
        events.push(parse_event_jsonl_line(line)?);
    }
    Ok(replay_trace_events(&events, options))
}

pub fn replay_from_checkpoint(
    plan: &PlanDocument,
    checkpoint: &CheckpointDocument,
    router: &RouterExecutor,
    solver: &dyn Solver,
    runner_options: &EngineRunnerOptions,
    replay_options: &ReplayOptions,
) -> ReplayResult {
    let mut state = EngineRunnerState {
        runtime: checkpoint
            .runtime_snapshot
            .clone()
            .unwrap_or(Value::Object(serde_json::Map::new())),
        completed_node_ids: checkpoint.engine_state.completed_node_ids.clone(),
        approved_node_ids: Vec::new(),
        seen_command_ids: checkpoint.engine_state.seen_command_ids.clone(),
        paused_reason: checkpoint.engine_state.paused_reason.clone(),
        pending_retries: checkpoint.engine_state.pending_retries.clone(),
        next_seq: 0,
    };

    if let Some(until_node) = replay_options.until_node.as_deref() {
        if state.completed_node_ids.iter().any(|id| id == until_node) {
            return ReplayResult {
                status: ReplayStatus::ReachedUntilNode,
                events: Vec::new(),
                completed_node_ids: state.completed_node_ids,
                paused_reason: state.paused_reason,
            };
        }
    }

    let mut all_events = Vec::<EngineEventRecord>::new();
    let mut previous_completed_count = state.completed_node_ids.len();
    let max_steps = replay_options.max_steps.max(1);

    for _ in 0..max_steps {
        let run_result = run_plan_once(
            checkpoint.run_id.as_str(),
            plan,
            &mut state,
            router,
            solver,
            &[],
            runner_options,
        );
        all_events.extend(run_result.events);

        if let Some(until_node) = replay_options.until_node.as_deref() {
            if state.completed_node_ids.iter().any(|id| id == until_node) {
                return ReplayResult {
                    status: ReplayStatus::ReachedUntilNode,
                    events: all_events,
                    completed_node_ids: state.completed_node_ids,
                    paused_reason: state.paused_reason,
                };
            }
        }

        match run_result.status {
            EngineRunStatus::Completed => {
                return ReplayResult {
                    status: ReplayStatus::Completed,
                    events: all_events,
                    completed_node_ids: state.completed_node_ids,
                    paused_reason: state.paused_reason,
                }
            }
            EngineRunStatus::Stopped => {
                return ReplayResult {
                    status: ReplayStatus::Paused,
                    events: all_events,
                    completed_node_ids: state.completed_node_ids,
                    paused_reason: state.paused_reason,
                };
            }
            EngineRunStatus::Paused => {
                if state.completed_node_ids.len() == previous_completed_count {
                    return ReplayResult {
                        status: ReplayStatus::Paused,
                        events: all_events,
                        completed_node_ids: state.completed_node_ids,
                        paused_reason: state.paused_reason,
                    };
                }
                previous_completed_count = state.completed_node_ids.len();
            }
        }
    }

    ReplayResult {
        status: ReplayStatus::Paused,
        events: all_events,
        completed_node_ids: state.completed_node_ids,
        paused_reason: Some("replay_step_limit".to_string()),
    }
}

#[cfg(test)]
#[path = "replay_test.rs"]
mod tests;
