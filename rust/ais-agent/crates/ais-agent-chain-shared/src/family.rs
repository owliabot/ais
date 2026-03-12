use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainFamily {
    Evm,
    Solana,
    Other(String),
}

impl ChainFamily {
    pub fn from_prefix(prefix: &str) -> Self {
        match prefix {
            "eip155" => Self::Evm,
            "solana" => Self::Solana,
            other => Self::Other(other.to_owned()),
        }
    }
}
