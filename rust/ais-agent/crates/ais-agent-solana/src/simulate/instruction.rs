//! Solana instruction simulation entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, SimulationCapability,
    SimulationRequest, SimulationResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaInstructionSimulator;

impl SimulationCapability for SolanaInstructionSimulator {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::Simulate,
            implementation: "solana.instruction_simulator",
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
                "implementation": "solana.instruction_simulator",
                "mode": request.mode,
            }),
            state_delta_hint: Some(json!({"stub": true})),
        })
    }
}
