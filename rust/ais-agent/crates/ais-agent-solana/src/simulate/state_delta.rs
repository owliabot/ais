//! Solana state-delta estimation entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, SimulationCapability,
    SimulationRequest, SimulationResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaStateDeltaEstimator;

impl SimulationCapability for SolanaStateDeltaEstimator {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::Simulate,
            implementation: "solana.state_delta_estimator",
        }
    }

    fn simulate(
        &self,
        request: &SimulationRequest,
    ) -> Result<SimulationResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Solana {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "solana".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(SimulationResponse {
            accepted: true,
            payload: json!({
                "implementation": "solana.state_delta_estimator",
                "mode": request.mode,
            }),
            state_delta_hint: Some(json!({"estimate_only": true})),
        })
    }
}
