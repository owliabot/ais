use crate::standard::{StandardDriverOutput, StandardDriverRequest};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StandardDriverError {
    #[error("unsupported action selector: {0}")]
    UnsupportedAction(String),
    #[error("invalid driver output: {0}")]
    InvalidOutput(String),
}

pub trait StandardDriver {
    fn driver_id(&self) -> &'static str;
    fn supports_action(&self, action_selector: &str) -> bool;
    fn build(
        &self,
        request: &StandardDriverRequest,
    ) -> Result<StandardDriverOutput, StandardDriverError>;
}
