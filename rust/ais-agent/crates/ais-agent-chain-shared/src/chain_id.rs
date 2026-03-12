//! Shared chain identifier helpers.

use serde::{Deserialize, Serialize};

use crate::ChainFamily;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChainId(pub String);

impl ChainId {
    pub fn family(&self) -> ChainFamily {
        let prefix = self.0.split(':').next().unwrap_or_default();
        ChainFamily::from_prefix(prefix)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ChainId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ChainId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}
