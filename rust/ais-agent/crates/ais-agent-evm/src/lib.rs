//! EVM chain-family capabilities and reflection builders.

pub mod artifact_planner;
pub mod broadcast;
pub mod read;
pub mod receipt;
pub mod reflect;
pub mod simulate;
pub mod state;

use ais_agent_chain_shared::{
    BroadcastCapability, ChainCapability, ChainFamily, ChainFamilySurface, ReadCapability,
    ReceiptCapability, SimulationCapability, StateCapability,
};

use crate::{
    broadcast::tx::EvmBroadcaster,
    read::{indexer::EvmIndexerReader, rpc::EvmRpcReader},
    receipt::{reconcile::EvmReceiptReconciler, watcher::EvmReceiptWatcher},
    simulate::{call::EvmCallSimulator, state_delta::EvmStateDeltaEstimator},
    state::EvmStateReader,
};

#[derive(Debug, Clone, Default)]
pub struct EvmChainSurface {
    pub rpc_reader: EvmRpcReader,
    pub indexer_reader: EvmIndexerReader,
    pub call_simulator: EvmCallSimulator,
    pub state_delta_estimator: EvmStateDeltaEstimator,
    pub broadcaster: EvmBroadcaster,
    pub receipt_watcher: EvmReceiptWatcher,
    pub receipt_reconciler: EvmReceiptReconciler,
    pub state_reader: EvmStateReader,
}

impl ChainFamilySurface for EvmChainSurface {
    fn family(&self) -> ChainFamily {
        ChainFamily::Evm
    }

    fn capabilities(&self) -> Vec<ChainCapability> {
        vec![
            self.rpc_reader.capability(),
            self.indexer_reader.capability(),
            self.call_simulator.capability(),
            self.state_delta_estimator.capability(),
            self.broadcaster.capability(),
            self.receipt_watcher.capability(),
            self.receipt_reconciler.capability(),
            self.state_reader.capability(),
        ]
    }
}
