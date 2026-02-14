use super::{parse_action_ref, parse_query_ref, resolve_action_ref, resolve_query_ref, ReferenceError};
use crate::documents::ProtocolDocument;
use crate::resolver::ResolverContext;
use serde_json::{json, Map, Value};

fn protocol_document() -> ProtocolDocument {
    let mut actions = Map::new();
    actions.insert("swap".to_string(), json!({"id":"swap"}));

    let mut queries = Map::new();
    queries.insert("quote".to_string(), json!({"id":"quote"}));

    ProtocolDocument {
        schema: "ais/0.0.2".to_string(),
        meta: json!({"protocol":"uniswap-v3","version":"1.0.0"}),
        deployments: vec![],
        actions,
        queries,
        risks: vec![],
        supported_assets: vec![],
        capabilities_required: vec![],
        tests: vec![],
        extensions: Map::<String, Value>::new(),
    }
}

#[test]
fn parse_action_reference() {
    let parsed = parse_action_ref("uniswap-v3@1.0.0/swap").expect("must parse");
    assert_eq!(parsed.protocol, "uniswap-v3");
    assert_eq!(parsed.version, "1.0.0");
    assert_eq!(parsed.action, "swap");
}

#[test]
fn parse_query_reference() {
    let parsed = parse_query_ref("uniswap-v3@1.0.0/quote").expect("must parse");
    assert_eq!(parsed.protocol, "uniswap-v3");
    assert_eq!(parsed.version, "1.0.0");
    assert_eq!(parsed.query, "quote");
}

#[test]
fn resolve_action_reference() {
    let mut context = ResolverContext::new();
    context.register_protocol(protocol_document());

    let resolved = resolve_action_ref(&context, "uniswap-v3@1.0.0/swap").expect("resolve");
    assert_eq!(resolved.reference.action, "swap");
    assert_eq!(resolved.action_spec["id"], "swap");
}

#[test]
fn resolve_query_reference() {
    let mut context = ResolverContext::new();
    context.register_protocol(protocol_document());

    let resolved = resolve_query_ref(&context, "uniswap-v3@1.0.0/quote").expect("resolve");
    assert_eq!(resolved.reference.query, "quote");
    assert_eq!(resolved.query_spec["id"], "quote");
}

#[test]
fn invalid_reference_returns_error() {
    let error = parse_action_ref("bad-ref").expect_err("must fail");
    assert!(matches!(error, ReferenceError::InvalidFormat { .. }));
}
