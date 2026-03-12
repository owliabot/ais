//! EVM call simulation entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, SimulationCapability,
    SimulationRequest, SimulationResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmCallSimulator;

impl SimulationCapability for EvmCallSimulator {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Evm,
            kind: CapabilityKind::Simulate,
            implementation: "evm.call_simulator",
        }
    }

    fn simulate(
        &self,
        request: &SimulationRequest,
    ) -> Result<SimulationResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Evm {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "evm".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(SimulationResponse {
            accepted: true,
            payload: json!({
                "implementation": "evm.call_simulator",
                "mode": request.mode,
            }),
            state_delta_hint: Some(json!({"stub": true})),
        })
    }
}
