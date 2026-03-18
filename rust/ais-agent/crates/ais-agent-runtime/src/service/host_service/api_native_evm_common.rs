#[derive(Debug, Clone, Default)]
pub struct RuntimeExecutionWiring {
    pub evm_rpc_url: Option<String>,
    pub solana_rpc_url: Option<String>,
}
