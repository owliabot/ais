//! EVM transaction broadcast entry points.

use ais_agent_chain_shared::{
    BroadcastCapability, BroadcastRequest, BroadcastResponse, CapabilityKind, ChainCapability,
    ChainCapabilityError, ChainFamily,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmBroadcaster;

impl BroadcastCapability for EvmBroadcaster {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Evm,
            kind: CapabilityKind::Broadcast,
            implementation: "evm.broadcaster",
        }
    }

    fn broadcast(
        &self,
        request: &BroadcastRequest,
    ) -> Result<BroadcastResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Evm {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "evm".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(BroadcastResponse {
            tx_hash: format!("stub-evm-{}", request.chain_id.as_str()),
            accepted_by: Some("stub:evm_broadcast".to_owned()),
        })
    }
}
