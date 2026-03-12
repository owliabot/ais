use crate::{
    BroadcastRequest, BroadcastResponse, ChainCapability, ChainCapabilityError, ChainFamily,
    ReadRequest, ReadResponse, ReceiptQuery, ReceiptView, SimulationRequest, SimulationResponse,
    StateQuery, StateView,
};

pub trait ReadCapability {
    fn capability(&self) -> ChainCapability;
    fn read(&self, request: &ReadRequest) -> Result<ReadResponse, ChainCapabilityError>;
}

pub trait SimulationCapability {
    fn capability(&self) -> ChainCapability;
    fn simulate(
        &self,
        request: &SimulationRequest,
    ) -> Result<SimulationResponse, ChainCapabilityError>;
}

pub trait BroadcastCapability {
    fn capability(&self) -> ChainCapability;
    fn broadcast(
        &self,
        request: &BroadcastRequest,
    ) -> Result<BroadcastResponse, ChainCapabilityError>;
}

pub trait ReceiptCapability {
    fn capability(&self) -> ChainCapability;
    fn receipt(&self, query: &ReceiptQuery) -> Result<ReceiptView, ChainCapabilityError>;
}

pub trait StateCapability {
    fn capability(&self) -> ChainCapability;
    fn state(&self, query: &StateQuery) -> Result<StateView, ChainCapabilityError>;
}

pub trait ChainFamilySurface {
    fn family(&self) -> ChainFamily;
    fn capabilities(&self) -> Vec<ChainCapability>;
}
