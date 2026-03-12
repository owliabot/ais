use super::{ApiNativeOutput, ApiNativeRequest};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiNativeAdapterError {
    #[error("unsupported provider kind")]
    UnsupportedProviderKind,
    #[error("missing direct envelope payload")]
    MissingDirectEnvelope,
    #[error("missing chain for api-native normalization")]
    MissingChain,
}

pub trait ApiNativeAdapter {
    fn adapter_id(&self) -> &'static str;
    fn build(&self, request: &ApiNativeRequest) -> Result<ApiNativeOutput, ApiNativeAdapterError>;
}
