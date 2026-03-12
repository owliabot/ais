//! Action-graph domain objects.

mod graph;
pub mod kinds;
mod node;

pub use graph::ActionGraph;
pub use node::{
    ActionInputRef, ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
};
