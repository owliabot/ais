//! Runtime-owned orchestration state.

mod active_run;
mod driver_binding;
mod patch;
mod recovery;
mod repository;
mod state_machine;

pub use active_run::ActiveRun;
pub use driver_binding::{
    DriverBindingContext, RawEnvelopeBindingRequest, RuntimeDriverBinder, RuntimeDriverBindingError,
};
pub use patch::{apply_plan_patch, RuntimePatchError, RuntimePatchOutcome};
pub use recovery::{
    classify_allowed_recovery_actions, classify_recovery_disposition,
    classify_recovery_suggestions, classify_recovery_view, classify_validated_recovery_view,
    validate_checkpoint_recovery_contract,
};
pub use repository::{InMemoryRunRepository, RunRepository, RunRepositoryError};
pub use state_machine::{CheckpointPersistenceMode, RuntimeStateMachine};
