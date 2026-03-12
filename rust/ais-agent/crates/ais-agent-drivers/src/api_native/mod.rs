//! API-native path for quote, route, and direct-envelope providers.

mod direct_envelope;
mod output;
mod quote;
mod request;
mod route;
mod trait_def;

pub use direct_envelope::DirectEnvelopeApiAdapter;
pub use output::{
    normalize_direct_envelope, normalize_quote_evidence, normalize_route_evidence, ApiNativeOutput,
    NativeEnvelopeArtifact,
};
pub use quote::QuoteApiAdapter;
pub use request::{
    ApiNativeProviderKind, ApiNativeRequest, DirectEnvelopePayload, EvmNativeEnvelope,
    SolanaNativeEnvelope,
};
pub use route::RouteApiAdapter;
pub use trait_def::{ApiNativeAdapter, ApiNativeAdapterError};

#[cfg(test)]
mod tests;
