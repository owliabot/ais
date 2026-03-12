use crate::api_native::{
    normalize_direct_envelope, ApiNativeAdapter, ApiNativeAdapterError, ApiNativeOutput,
    ApiNativeProviderKind, ApiNativeRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirectEnvelopeApiAdapter;

impl ApiNativeAdapter for DirectEnvelopeApiAdapter {
    fn adapter_id(&self) -> &'static str {
        "api_native.direct_envelope"
    }

    fn build(&self, request: &ApiNativeRequest) -> Result<ApiNativeOutput, ApiNativeAdapterError> {
        if request.provider_kind != ApiNativeProviderKind::DirectEnvelopeProvider {
            return Err(ApiNativeAdapterError::UnsupportedProviderKind);
        }

        let chain = request
            .chain
            .as_deref()
            .ok_or(ApiNativeAdapterError::MissingChain)?;
        let direct = request
            .direct_envelope
            .clone()
            .ok_or(ApiNativeAdapterError::MissingDirectEnvelope)?;

        let (runtime_envelope, native_envelope, fragment, effect_contract) =
            normalize_direct_envelope(request.provider_id.as_str(), chain, direct);

        Ok(ApiNativeOutput {
            runtime_envelopes: vec![runtime_envelope],
            native_envelopes: vec![native_envelope],
            fragment,
            effect_contracts: vec![effect_contract],
            ..ApiNativeOutput::default()
        })
    }
}
