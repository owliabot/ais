use std::str::FromStr;

use alloy::primitives::{Address, U256};

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

pub(super) fn parse_address(value: &str, field: &str) -> Result<Address, String> {
    Address::from_str(value).map_err(|error| format!("invalid {field} address `{value}`: {error}"))
}

pub(super) fn parse_u256_decimal(value: &str) -> Result<U256, String> {
    U256::from_str_radix(value, 10)
        .map_err(|error| format!("invalid atomic amount `{value}`: {error}"))
}
