use std::collections::BTreeMap;

use ais_agent_core::{
    envelope::RuntimeEnvelopeKind,
    evidence::{EvidenceGraph, EvidenceKind},
    mission::{Mission, MissionBudget, MissionPolicy},
};
use alloy_primitives::{Address, Bytes, U256};
use serde_json::json;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey};

use crate::api_native::{
    ApiNativeAdapter, ApiNativeProviderKind, ApiNativeRequest, DirectEnvelopeApiAdapter,
    DirectEnvelopePayload, EvmNativeEnvelope, QuoteApiAdapter, RouteApiAdapter,
    SolanaNativeEnvelope,
};

#[test]
fn quote_adapter_normalizes_quote_into_evidence() {
    let adapter = QuoteApiAdapter;
    let output = adapter
        .build(&ApiNativeRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            provider_id: "quote-provider".to_owned(),
            provider_kind: ApiNativeProviderKind::QuoteProvider,
            chain: Some("eip155:1".to_owned()),
            payload: json!({"amount_out":"1000"}),
            direct_envelope: None,
        })
        .expect("quote output");

    assert_eq!(output.evidence_records.len(), 1);
    assert_eq!(output.evidence_records[0].kind, EvidenceKind::RouteOrQuote);
    assert!(output.runtime_envelopes.is_empty());
}

#[test]
fn route_adapter_normalizes_route_into_evidence() {
    let adapter = RouteApiAdapter;
    let output = adapter
        .build(&ApiNativeRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            provider_id: "route-provider".to_owned(),
            provider_kind: ApiNativeProviderKind::RouteProvider,
            chain: Some("eip155:1".to_owned()),
            payload: json!({"path":["USDC","WETH"]}),
            direct_envelope: None,
        })
        .expect("route output");

    assert_eq!(output.evidence_records.len(), 1);
    assert_eq!(output.evidence_records[0].kind, EvidenceKind::RouteOrQuote);
}

#[test]
fn direct_envelope_adapter_normalizes_native_evm_envelope() {
    let adapter = DirectEnvelopeApiAdapter;
    let output = adapter
        .build(&ApiNativeRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            provider_id: "direct-provider".to_owned(),
            provider_kind: ApiNativeProviderKind::DirectEnvelopeProvider,
            chain: Some("eip155:1".to_owned()),
            payload: json!({"provider":"direct"}),
            direct_envelope: Some(DirectEnvelopePayload::Evm(EvmNativeEnvelope {
                to: Address::from([0x11; 20]),
                data: Bytes::from_static(b"\xde\xad"),
                value: U256::from(0u64),
            })),
        })
        .expect("direct envelope output");

    assert_eq!(output.runtime_envelopes.len(), 1);
    assert_eq!(
        output.runtime_envelopes[0].kind,
        RuntimeEnvelopeKind::EvmEnvelope
    );
    assert_eq!(output.native_envelopes.len(), 1);
    assert_eq!(output.effect_contracts.len(), 1);
    assert_eq!(
        output.fragment.roots,
        vec!["api_native.direct-provider.simulate"]
    );
    assert_eq!(
        output.fragment.terminals,
        vec!["api_native.direct-provider.verify"]
    );
    assert!(output
        .fragment
        .nodes
        .contains_key("api_native.direct-provider.actuate"));
}

#[test]
fn direct_envelope_adapter_normalizes_native_solana_envelope() {
    let adapter = DirectEnvelopeApiAdapter;
    let output = adapter
        .build(&ApiNativeRequest {
            mission: sample_mission(),
            evidence: EvidenceGraph::default(),
            provider_id: "solana-direct".to_owned(),
            provider_kind: ApiNativeProviderKind::DirectEnvelopeProvider,
            chain: Some("solana:mainnet".to_owned()),
            payload: json!({"provider":"direct"}),
            direct_envelope: Some(DirectEnvelopePayload::Solana(SolanaNativeEnvelope {
                instructions: vec![Instruction {
                    program_id: Pubkey::new_from_array([1u8; 32]),
                    accounts: Vec::new(),
                    data: vec![1, 2, 3],
                }],
            })),
        })
        .expect("direct envelope output");

    assert_eq!(output.runtime_envelopes.len(), 1);
    assert_eq!(
        output.runtime_envelopes[0].kind,
        RuntimeEnvelopeKind::SolanaEnvelope
    );
    assert_eq!(output.effect_contracts.len(), 1);
    assert_eq!(
        output.fragment.roots,
        vec!["api_native.solana-direct.actuate"]
    );
    assert_eq!(
        output.fragment.terminals,
        vec!["api_native.solana-direct.verify"]
    );
}

fn sample_mission() -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "normalize provider outputs".to_owned(),
        allowed_chains: vec!["eip155:1".to_owned(), "solana:mainnet".to_owned()],
        budget: MissionBudget::default(),
        policy: MissionPolicy::default(),
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}
