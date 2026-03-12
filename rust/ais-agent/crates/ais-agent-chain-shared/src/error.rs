use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChainCapabilityError {
    #[error("unsupported chain family for capability: expected {expected}, got {actual}")]
    UnsupportedChainFamily { expected: String, actual: String },
    #[error("capability not implemented: {0}")]
    NotImplemented(&'static str),
}
