//! EVM RPC read entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, ReadCapability,
    ReadRequest, ReadResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmRpcReader;

impl ReadCapability for EvmRpcReader {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Evm,
            kind: CapabilityKind::Read,
            implementation: "evm.rpc_reader",
        }
    }

    fn read(&self, request: &ReadRequest) -> Result<ReadResponse, ChainCapabilityError> {
        if request.chain_id.family() != ChainFamily::Evm {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "evm".to_owned(),
                actual: request.chain_id.as_str().to_owned(),
            });
        }

        Ok(ReadResponse {
            payload: json!({
                "implementation": "evm.rpc_reader",
                "method": request.method,
            }),
            source_hint: Some("stub:evm_rpc".to_owned()),
        })
    }
}
