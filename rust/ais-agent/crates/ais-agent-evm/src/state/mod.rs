//! EVM chain-state projections.

pub mod allowance;
pub mod position;
pub mod wallet;

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, StateCapability,
    StateQuery, StateView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmStateReader;

impl StateCapability for EvmStateReader {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Evm,
            kind: CapabilityKind::State,
            implementation: "evm.state_reader",
        }
    }

    fn state(&self, query: &StateQuery) -> Result<StateView, ChainCapabilityError> {
        if query.chain_id.family() != ChainFamily::Evm {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "evm".to_owned(),
                actual: query.chain_id.as_str().to_owned(),
            });
        }

        Ok(StateView {
            subject: query.subject.clone(),
            observed_at_ms: None,
            payload: json!({
                "implementation": "evm.state_reader",
                "subject": query.subject,
            }),
        })
    }
}
