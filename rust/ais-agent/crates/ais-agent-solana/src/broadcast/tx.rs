//! Solana transaction broadcast entry points.

use ais_agent_chain_shared::{
    BroadcastCapability, BroadcastRequest, BroadcastResponse, CapabilityKind, ChainCapability,
    ChainCapabilityError, ChainFamily,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaBroadcaster;

impl BroadcastCapability for SolanaBroadcaster {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::Broadcast,
            implementation: "solana.broadcaster",
        }
    }

    fn broadcast(
        &self,
        request: &BroadcastRequest,
    ) -> Result<BroadcastResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Solana {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "solana".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(BroadcastResponse {
            tx_hash: format!("stub-solana-{}", request.chain_id.as_str()),
            accepted_by: Some("stub:solana_broadcast".to_owned()),
        })
    }
}
