//! Solana chain-family capabilities.

pub mod broadcast;
pub mod read;
pub mod receipt;
pub mod simulate;
pub mod state;

use ais_agent_chain_shared::{ChainCapability, ChainFamily, ChainFamilySurface};

use self::{
    broadcast::tx::SolanaBroadcaster,
    read::{indexer::SolanaIndexerReader, rpc::SolanaRpcReader},
    receipt::{reconcile::SolanaReceiptReconciler, watcher::SolanaReceiptWatcher},
    simulate::{instruction::SolanaInstructionSimulator, state_delta::SolanaStateDeltaEstimator},
    state::SolanaStateReader,
};

#[derive(Debug, Clone, Default)]
pub struct SolanaChainSurface {
    pub rpc_reader: SolanaRpcReader,
    pub indexer_reader: SolanaIndexerReader,
    pub instruction_simulator: SolanaInstructionSimulator,
    pub state_delta_estimator: SolanaStateDeltaEstimator,
    pub broadcaster: SolanaBroadcaster,
    pub receipt_watcher: SolanaReceiptWatcher,
    pub receipt_reconciler: SolanaReceiptReconciler,
    pub state_reader: SolanaStateReader,
}

impl ChainFamilySurface for SolanaChainSurface {
    fn family(&self) -> ChainFamily {
        ChainFamily::Solana
    }

    fn capabilities(&self) -> Vec<ChainCapability> {
        use ais_agent_chain_shared::{
            BroadcastCapability, ReadCapability, ReceiptCapability, SimulationCapability,
            StateCapability,
        };

        vec![
            self.rpc_reader.capability(),
            self.indexer_reader.capability(),
            self.instruction_simulator.capability(),
            self.state_delta_estimator.capability(),
            self.broadcaster.capability(),
            self.receipt_watcher.capability(),
            self.receipt_reconciler.capability(),
            self.state_reader.capability(),
        ]
    }
}
