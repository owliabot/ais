//! Completion transition.

use ais_agent_core::{
    action::ActionNodeStatus,
    runtime::{RunPhase, RunStatus},
};

use crate::{
    runtime::ActiveRun,
    stepper::{StepTransition, StepTransitionKind},
};

pub(crate) fn apply_complete_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    if matches!(
        runtime.checkpoint.lifecycle.status,
        RunStatus::Completed | RunStatus::Cancelled
    ) {
        return None;
    }
    if runtime.pending_signer_state.is_some()
        || !runtime
            .checkpoint
            .pending_requests
            .pending_evidence_refs
            .is_empty()
        || runtime
            .checkpoint
            .pending_requests
            .pending_signer_request_id
            .is_some()
    {
        return None;
    }
    if runtime
        .checkpoint
        .execution_artifact
        .as_ref()
        .is_some_and(|snapshot| {
            snapshot.active_stage_id.is_some() || snapshot.awaiting_continuation.is_some()
        })
    {
        return None;
    }
    if runtime.checkpoint.action_graph.terminals.is_empty() {
        return None;
    }

    let all_terminal_nodes_complete =
        runtime
            .checkpoint
            .action_graph
            .terminals
            .iter()
            .all(|node_id| {
                runtime
                    .checkpoint
                    .action_graph
                    .nodes
                    .get(node_id)
                    .is_some_and(|node| {
                        matches!(
                            node.status,
                            ActionNodeStatus::Succeeded | ActionNodeStatus::Skipped
                        )
                    })
            });
    if !all_terminal_nodes_complete {
        return None;
    }

    runtime
        .checkpoint
        .lifecycle
        .complete("all terminal nodes completed");
    runtime.checkpoint.lifecycle.phase = RunPhase::Finalized;
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Complete,
        node_id: None,
        summary: "run completed".to_owned(),
    })
}
