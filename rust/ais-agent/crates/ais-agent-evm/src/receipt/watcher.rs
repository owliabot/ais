//! EVM receipt watcher entry points.

use serde_json::json;

use ais_agent_chain_shared::{
    CapabilityKind, ChainCapability, ChainCapabilityError, ChainFamily, ConfirmationDepth,
    FinalityLevel, ReceiptCapability, ReceiptQuery, ReceiptView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvmReceiptWatcher;

impl ReceiptCapability for EvmReceiptWatcher {
    fn capability(&self) -> ChainCapability {
        ChainCapability {
            family: ChainFamily::Evm,
            kind: CapabilityKind::Receipt,
            implementation: "evm.receipt_watcher",
        }
    }

    fn receipt(&self, query: &ReceiptQuery) -> Result<ReceiptView, ChainCapabilityError> {
        if query.chain_id.family() != ChainFamily::Evm {
            return Err(ChainCapabilityError::UnsupportedChainFamily {
                expected: "evm".to_owned(),
                actual: query.chain_id.as_str().to_owned(),
            });
        }

        Ok(ReceiptView {
            tx_hash: query.tx_hash.clone(),
            finality: FinalityLevel::Observed,
            confirmation_depth: query.min_confirmation_depth.or(Some(ConfirmationDepth(0))),
            payload: json!({"implementation": "evm.receipt_watcher"}),
        })
    }
}
