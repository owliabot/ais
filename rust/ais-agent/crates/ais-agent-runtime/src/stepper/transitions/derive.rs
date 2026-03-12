//! Derived-value transition.

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

pub(crate) fn apply_derive_transition(runtime: &mut ActiveRun) -> Option<StepTransition> {
    let node_id = runtime
        .checkpoint
        .action_graph
        .nodes
        .iter()
        .find(|(_, node)| {
            node.kind == ActionNodeKind::Derive
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
        .mark_running(RunPhase::Planning);
    runtime.touch_transition();

    Some(StepTransition {
        kind: StepTransitionKind::Derive,
        node_id: Some(node_id.clone()),
        summary: format!("completed derive node {node_id}"),
    })
}
