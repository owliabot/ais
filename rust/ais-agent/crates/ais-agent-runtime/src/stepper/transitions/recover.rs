//! Recovery transition.

use ais_agent_core::{
    action::{ActionNodeKind, ActionNodeStatus},
    runtime::RunPhase,
};

use crate::{
    runtime::ActiveRun,
    stepper::{
        transitions::{dependencies_satisfied, mark_node_status},
        StepTransition, StepTransitionKind,
    },
};

pub(crate) fn apply_recover_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let has_failed_node = runtime
        .checkpoint
        .action_graph
        .nodes
        .values()
        .any(|node| node.status == ActionNodeStatus::Failed);
    if !has_failed_node {
        return None;
    }

    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Recover
                && matches!(
                    node.status,
                    ActionNodeStatus::Pending | ActionNodeStatus::Ready
                )
                && dependencies_satisfied(&runtime.checkpoint.action_graph, node)
        })
        .map(|(node_id, _)| node_id.clone())?;

    mark_node_status(runtime, node_id.as_str(), ActionNodeStatus::Succeeded);
    runtime
        .checkpoint
        .lifecycle
        .mark_running(RunPhase::Recovering);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Recover,
        node_id: Some(node_id.clone()),
        summary: format!("executed recovery node {node_id}"),
    })
}
