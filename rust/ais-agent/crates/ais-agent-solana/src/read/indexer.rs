//! Solana indexer-backed read entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, ReadCapability,
    ReadRequest, ReadResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaIndexerReader;

impl ReadCapability for SolanaIndexerReader {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::Read,
            implementation: "solana.indexer_reader",
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
                "implementation": "solana.indexer_reader",
                "method": request.method,
            }),
            source_hint: Some("stub:solana_indexer".to_owned()),
        })
    }
}
