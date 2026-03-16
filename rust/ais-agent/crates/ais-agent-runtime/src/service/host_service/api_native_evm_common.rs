#[derive(Debug, Clone, Default)]
pub struct RuntimeExecutionWiring {
    pub evm_rpc_url: Option<String>,
    pub solana_rpc_url: Option<String>,
    pub allowed_protocol_packages: Vec<String>,
}

impl RuntimeExecutionWiring {
    pub fn allows_protocol_package(&self, protocol_package_id: &str) -> bool {
        self.allowed_protocol_packages
            .iter()
            .any(|allowed| allowed == protocol_package_id)
    }
}
