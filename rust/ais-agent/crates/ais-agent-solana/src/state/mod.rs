//! Solana chain-state projections.

pub mod position;
pub mod token;

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, StateCapability,
    StateQuery, StateView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaStateReader;

impl StateCapability for SolanaStateReader {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Solana,
            kind: CapabilityKind::State,
            implementation: "solana.state_reader",
        }
    }

    fn state(&self, query: &StateQuery) -> Result<StateView, ChainCapabilityError> {
        if query.chain_id.family() != ChainFamily::Solana {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "solana".to_owned(),
                actual: query.chain_id.as_str().to_owned(),
            });
        }

        Ok(StateView {
            subject: query.subject.clone(),
            observed_at_ms: None,
            payload: json!({
                "implementation": "solana.state_reader",
                "subject": query.subject,
            }),
        })
    }
}
