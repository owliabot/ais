use crate::api_native::{
    normalize_route_evidence, ApiNativeAdapter, ApiNativeAdapterError, ApiNativeOutput,
    ApiNativeProviderKind, ApiNativeRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteApiAdapter;

impl ApiNativeAdapter for RouteApiAdapter {
    fn adapter_id(&self) -> &'static str {
        "api_native.route"
    }

    fn build(&self, request: &ApiNativeRequest) -> Result<ApiNativeOutput, ApiNativeAdapterError> {
        if request.provider_kind != ApiNativeProviderKind::RouteProvider {
            return Err(ApiNativeAdapterError::UnsupportedProviderKind);
        }

        Ok(ApiNativeOutput {
            evidence_records: vec![normalize_route_evidence(
                request.provider_id.as_str(),
                request.chain.as_deref(),
                request.payload.clone(),
            )],
            ..ApiNativeOutput::default()
        })
    }
}
