use crate::api_native::{
    normalize_quote_evidence, ApiNativeAdapter, ApiNativeAdapterError, ApiNativeOutput,
    ApiNativeProviderKind, ApiNativeRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuoteApiAdapter;

impl ApiNativeAdapter for QuoteApiAdapter {
    fn adapter_id(&self) -> &'static str {
        "api_native.quote"
    }

    fn build(&self, request: &ApiNativeRequest) -> Result<ApiNativeOutput, ApiNativeAdapterError> {
        if request.provider_kind != ApiNativeProviderKind::QuoteProvider {
            return Err(ApiNativeAdapterError::UnsupportedProviderKind);
        }

        Ok(ApiNativeOutput {
            evidence_records: vec![normalize_quote_evidence(
                request.provider_id.as_str(),
                request.chain.as_deref(),
                request.payload.clone(),
            )],
            ..ApiNativeOutput::default()
        })
    }
}
