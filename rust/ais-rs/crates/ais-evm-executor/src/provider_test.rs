use super::{EvmProviderRegistry, EvmRpcEndpoint, ProviderError};

#[test]
fn registry_builds_for_configured_chain() {
    let endpoint = EvmRpcEndpoint::new("eip155:1", "https://eth.example").expect("endpoint");
    let registry = EvmProviderRegistry::from_endpoints(vec![endpoint]).expect("registry");
    let loaded = registry.endpoint("eip155:1").expect("must resolve");
    assert_eq!(loaded.rpc_url, "https://eth.example");
}

#[test]
fn registry_rejects_duplicate_chain() {
    let duplicate = EvmProviderRegistry::from_endpoints(vec![
        EvmRpcEndpoint::new("eip155:1", "https://a.example").expect("endpoint"),
        EvmRpcEndpoint::new("eip155:1", "https://b.example").expect("endpoint"),
    ])
    .expect_err("must reject");
    assert!(matches!(duplicate, ProviderError::DuplicateChain(chain) if chain == "eip155:1"));
}

#[test]
fn endpoint_rejects_invalid_chain_or_url() {
    let chain_error = EvmRpcEndpoint::new("solana:mainnet", "https://eth.example")
        .expect_err("must reject chain");
    assert!(matches!(chain_error, ProviderError::InvalidChain(_)));

    let url_error = EvmRpcEndpoint::new("eip155:1", "ftp://eth.example").expect_err("must reject url");
    assert!(matches!(url_error, ProviderError::InvalidRpcUrl(_)));
}

#[test]
fn endpoint_accepts_ws_and_detects_transport() {
    let endpoint = EvmRpcEndpoint::new("eip155:1", "wss://eth.example").expect("wss endpoint");
    assert_eq!(
        endpoint.transport().expect("transport"),
        super::EvmRpcTransport::Ws
    );
}
