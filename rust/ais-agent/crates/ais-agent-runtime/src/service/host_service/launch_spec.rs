use ais_agent_control::launch_spec::LaunchSpecSubmission;
use ais_agent_core::checkpoint::CheckpointSnapshot;

use super::{
    artifact_planner::seed_execution_artifact_checkpoint,
    launch_binding::seed_prebuilt_fragment_checkpoint,
    launch_validation::validate_launch_spec_submission, RuntimeExecutionWiring,
    RuntimeHostServiceError,
};

pub(crate) fn seed_launch_spec_checkpoint(
    checkpoint: &mut CheckpointSnapshot,
    wiring: &RuntimeExecutionWiring,
    launch_spec: &LaunchSpecSubmission,
) -> Result<(), RuntimeHostServiceError> {
    let validated_prebuilt = validate_launch_spec_submission(launch_spec)
        .map_err(RuntimeHostServiceError::invalid_command)?;

    match launch_spec {
        LaunchSpecSubmission::PrebuiltFragment(_) => {
            let validated = validated_prebuilt
                .expect("prebuilt_fragment validation must produce parsed fragment");
            seed_prebuilt_fragment_checkpoint(checkpoint, validated);
            Ok(())
        }
        LaunchSpecSubmission::ExecutionArtifact(spec) => {
            seed_execution_artifact_checkpoint(checkpoint, wiring, spec)
        }
        LaunchSpecSubmission::ReflectionRequest(_) => {
            Err(RuntimeHostServiceError::invalid_command(
                "reflection_request launch specs are not implemented yet",
            ))
        }
    }
}
