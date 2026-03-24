use serde::{Deserialize, Serialize};

use ais_agent_control::{
    events::RunEventEnvelope,
    ids::{ClaimId, RunId},
};

use crate::{
    inspect::{InspectSnapshot, PauseBundle},
    session::HostSessionSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostAcceptedResponse {
    pub run_id: Option<RunId>,
    pub message: Option<String>,
    pub session: Option<HostSessionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostErrorClass {
    NotFound,
    Conflict,
    InvalidCommand,
    Precondition,
    ProviderBinding,
    RecoveryContract,
    Ownership,
    Persistence,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostErrorRecoveryHints {
    #[serde(default)]
    pub requires_relink: bool,
    #[serde(default)]
    pub requires_patch: bool,
    #[serde(default)]
    pub requires_evidence: bool,
    #[serde(default)]
    pub requires_envelope: bool,
    #[serde(default)]
    pub operator_action_recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostErrorCorrelation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<ClaimId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostProviderBindingErrorContext {
    pub chain_scope: String,
    pub expected_family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_family: Option<String>,
    pub provider_lookup_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandError {
    pub code: String,
    pub message: String,
    pub error_class: HostErrorClass,
    pub retryable: bool,
    #[serde(default)]
    pub recovery_hints: HostErrorRecoveryHints,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<HostErrorCorrelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_binding: Option<HostProviderBindingErrorContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostCommandResponse {
    Accepted(HostAcceptedResponse),
    Inspect(InspectSnapshot),
    Pause(PauseBundle),
    Session(HostSessionSnapshot),
    Error(HostCommandError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCommandOutcome {
    pub response: HostCommandResponse,
    #[serde(default)]
    pub events: Vec<RunEventEnvelope>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_command_error_serializes_structured_fields() {
        let error = HostCommandError {
            code: "provider_not_configured".to_owned(),
            message: "provider binding failed".to_owned(),
            error_class: HostErrorClass::ProviderBinding,
            retryable: false,
            recovery_hints: HostErrorRecoveryHints {
                operator_action_recommended: true,
                ..HostErrorRecoveryHints::default()
            },
            correlation: Some(HostErrorCorrelation {
                run_id: Some(RunId("run-1".to_owned())),
                claim_id: Some(ClaimId("claim-1".to_owned())),
                checkpoint_seq: Some(7),
            }),
            provider_binding: Some(HostProviderBindingErrorContext {
                chain_scope: "eip155:8453".to_owned(),
                expected_family: "evm".to_owned(),
                actual_family: None,
                provider_lookup_scope: "runtime_execution_wiring.chains".to_owned(),
            }),
        };

        let json = serde_json::to_value(&error).expect("serialize error");
        assert_eq!(json["error_class"], "provider_binding");
        assert_eq!(json["correlation"]["claim_id"], "claim-1");
        assert_eq!(json["provider_binding"]["chain_scope"], "eip155:8453");
    }
}
