//! Host-facing inspect and pause projections.

pub mod pause_bundle;
pub mod pending;
pub mod progress;
pub mod projector;
pub mod snapshot;

#[cfg(test)]
mod tests;

pub use pause_bundle::{PauseActionView, PauseBundle, PauseKind};
pub use pending::{PendingConfirmationView, PendingContinuationView, PendingSignerRequestView};
pub use progress::{ActionStatusCountsView, ProgressView};
pub use projector::{
    project_inspect_snapshot, project_inspect_snapshot_with_recovery, project_pause_bundle,
    project_pause_bundle_with_recovery, project_progress_view,
};
pub use snapshot::{
    ActiveBoundaryView, BoundaryKind, EffectStatusView, InspectSnapshot, MissionSummaryView,
    RecoveryView, RequiredInputView, RunPhase, RunResultView, RunStatus, SideEffectView,
};
