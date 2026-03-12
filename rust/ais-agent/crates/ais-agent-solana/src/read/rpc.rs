//! Solana RPC read entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, ReadCapability,
    ReadRequest, ReadResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaRpcReader;

impl ReadCapability for SolanaRpcReader {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::Read,
            implementation: "solana.rpc_reader",
        }
    }

    fn read(&self, request: &ReadRequest) -> Result<ReadResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Solana {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "solana".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(ReadResponse {
            payload: json!({
                "implementation": "solana.rpc_reader",
                "method": request.method,
            }),
            source_hint: Some("stub:solana_rpc".to_owned()),
        })
    }
}
