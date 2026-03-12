use serde_json::json;

use ais_agent_core::{
    action::{
        kinds::{
            actuate::{ActuateAction, ActuateMode},
            simulate::{SimulateAction, SimulateKind},
            verify::{VerifyAction, VerifyKind},
        },
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    binding::evm::{EvmActuateBinding, EvmCallRequest, EvmSimulateBinding, EvmVerifyBinding},
    driver::{
        ActionGraphFragment, DriverEvmActuateHint, DriverEvmSimulateHint, DriverEvmVerifyHint,
        DriverNodeLiveBindingHint,
    },
    effect::{EffectAssertion, EffectContract, EffectContractKind},
    envelope::{RuntimeEnvelope, RuntimeEnvelopeKind},
    evidence::{EvidenceFreshness, EvidenceKind, EvidenceProvenance, EvidenceRecord},
};

use crate::api_native::{DirectEnvelopePayload, EvmNativeEnvelope, SolanaNativeEnvelope};

#[derive(Debug, Clone)]
pub enum NativeEnvelopeArtifact {
    Evm(EvmNativeEnvelope),
    Solana(SolanaNativeEnvelope),
    ExternalJob(serde_json::Value),
}

#[derive(Debug, Clone, Default)]
pub struct ApiNativeOutput {
    pub evidence_records: Vec<EvidenceRecord>,
    pub runtime_envelopes: Vec<RuntimeEnvelope>,
    pub native_envelopes: Vec<NativeEnvelopeArtifact>,
    pub fragment: ActionGraphFragment,
    pub effect_contracts: Vec<EffectContract>,
}

pub fn normalize_quote_evidence(
    provider_id: &str,
    chain: Option<&str>,
    payload: serde_json::Value,
) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: format!("evidence.{provider_id}.quote"),
        kind: EvidenceKind::RouteOrQuote,
        provenance: EvidenceProvenance {
            source: provider_id.to_owned(),
            chain_scope: chain.map(|value| value.to_owned()),
            trace_hint: None,
        },
        freshness: EvidenceFreshness::default(),
        confidence_ppm: Some(900_000),
        payload,
    }
}

pub fn normalize_route_evidence(
    provider_id: &str,
    chain: Option<&str>,
    payload: serde_json::Value,
) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: format!("evidence.{provider_id}.route"),
        kind: EvidenceKind::RouteOrQuote,
        provenance: EvidenceProvenance {
            source: provider_id.to_owned(),
            chain_scope: chain.map(|value| value.to_owned()),
            trace_hint: None,
        },
        freshness: EvidenceFreshness::default(),
        confidence_ppm: Some(850_000),
        payload,
    }
}

pub fn normalize_direct_envelope(
    provider_id: &str,
    chain: &str,
    payload: DirectEnvelopePayload,
) -> (
    RuntimeEnvelope,
    NativeEnvelopeArtifact,
    ActionGraphFragment,
    EffectContract,
) {
    match payload {
        DirectEnvelopePayload::Evm(native) => {
            let envelope = RuntimeEnvelope {
                envelope_id: format!("envelope.{provider_id}.evm"),
                kind: RuntimeEnvelopeKind::EvmEnvelope,
                chain: chain.to_owned(),
                payload: json!({
                    "normalized_from": "alloy_native_envelope",
                    "to": format!("{:?}", native.to),
                    "data": format!("{:?}", native.data),
                    "value": native.value.to_string(),
                    "raw_tx": "0xfeedbeef",
                }),
                provenance: Some(provider_id.to_owned()),
            };
            let effect_contract =
                direct_effect_contract(provider_id, "native evm envelope should produce receipt");
            let fragment =
                direct_evm_fragment(provider_id, chain, &envelope, &effect_contract, &native);
            (
                envelope,
                NativeEnvelopeArtifact::Evm(native),
                fragment,
                effect_contract,
            )
        }
        DirectEnvelopePayload::Solana(native) => {
            let envelope = RuntimeEnvelope {
                envelope_id: format!("envelope.{provider_id}.solana"),
                kind: RuntimeEnvelopeKind::SolanaEnvelope,
                chain: chain.to_owned(),
                payload: json!({
                    "normalized_from": "solana_sdk_native_envelope",
                    "instruction_count": native.instructions.len(),
                }),
                provenance: Some(provider_id.to_owned()),
            };
            let effect_contract = direct_effect_contract(
                provider_id,
                "native solana envelope should produce receipt",
            );
            (
                envelope.clone(),
                NativeEnvelopeArtifact::Solana(native),
                direct_generic_fragment(provider_id, chain, envelope, &effect_contract),
                effect_contract,
            )
        }
        DirectEnvelopePayload::ExternalJob(payload) => {
            let envelope = RuntimeEnvelope {
                envelope_id: format!("envelope.{provider_id}.external"),
                kind: RuntimeEnvelopeKind::ExternalJob,
                chain: chain.to_owned(),
                payload,
                provenance: Some(provider_id.to_owned()),
            };
            let effect_contract =
                direct_effect_contract(provider_id, "external job should report completion");
            (
                envelope.clone(),
                NativeEnvelopeArtifact::ExternalJob(json!({"provider_id": provider_id})),
                direct_generic_fragment(provider_id, chain, envelope, &effect_contract),
                effect_contract,
            )
        }
    }
}

fn direct_effect_contract(provider_id: &str, description: &str) -> EffectContract {
    EffectContract {
        effect_id: format!("effects.{provider_id}.direct_envelope"),
        kind: EffectContractKind::StateTransition,
        assertions: vec![EffectAssertion {
            expression: "receipt != null".to_owned(),
            description: description.to_owned(),
        }],
        tolerance_hint: Some("receipt_required".to_owned()),
    }
}

fn direct_evm_fragment(
    provider_id: &str,
    chain: &str,
    envelope: &RuntimeEnvelope,
    effect_contract: &EffectContract,
    native: &EvmNativeEnvelope,
) -> ActionGraphFragment {
    let simulate_id = format!("api_native.{provider_id}.simulate");
    let actuate_id = format!("api_native.{provider_id}.actuate");
    let verify_id = format!("api_native.{provider_id}.verify");

    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        simulate_id.clone(),
        ActionNode {
            node_id: simulate_id.clone(),
            kind: ActionNodeKind::Simulate,
            origin: ActionOrigin::ApiNativePath,
            status: ActionNodeStatus::Pending,
            depends_on: Vec::new(),
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Simulate(SimulateAction {
                simulate_kind: SimulateKind::Call,
                simulator_hint: format!("api-native {provider_id} eth_call"),
                live: None,
            }),
            implementation_hint: Some(provider_id.to_owned()),
            expected_effect_ref: None,
        },
    );
    nodes.insert(
        actuate_id.clone(),
        ActionNode {
            node_id: actuate_id.clone(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::ApiNativePath,
            status: ActionNodeStatus::Pending,
            depends_on: vec![simulate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Actuate(ActuateAction {
                mode: ActuateMode::ApiNativeEnvelope,
                actuator_hint: format!("api-native {provider_id} direct envelope"),
                chain: Some(chain.to_owned()),
                envelope_ref: Some(envelope.envelope_id.clone()),
                requires_effect_contract: true,
                live: None,
            }),
            implementation_hint: Some(provider_id.to_owned()),
            expected_effect_ref: Some(effect_contract.effect_id.clone()),
        },
    );
    nodes.insert(
        verify_id.clone(),
        ActionNode {
            node_id: verify_id.clone(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::ApiNativePath,
            status: ActionNodeStatus::Pending,
            depends_on: vec![actuate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: VerifyKind::EffectContract,
                verifier_hint: format!("api-native {provider_id} effect verify"),
                pre_observation_ref: None,
                post_observation_ref: None,
                live: None,
            }),
            implementation_hint: Some(provider_id.to_owned()),
            expected_effect_ref: Some(effect_contract.effect_id.clone()),
        },
    );

    let mut live_binding_hints = std::collections::BTreeMap::new();
    live_binding_hints.insert(
        simulate_id.clone(),
        DriverNodeLiveBindingHint::EvmSimulate(DriverEvmSimulateHint {
            binding: EvmSimulateBinding::EthCall,
            request: EvmCallRequest {
                from: None,
                to: native.to,
                data: native.data.clone(),
                value: Some(native.value),
            },
        }),
    );
    live_binding_hints.insert(
        actuate_id.clone(),
        DriverNodeLiveBindingHint::EvmActuate(DriverEvmActuateHint {
            binding: EvmActuateBinding::BroadcastRawTransaction,
        }),
    );
    live_binding_hints.insert(
        verify_id.clone(),
        DriverNodeLiveBindingHint::EvmVerify(DriverEvmVerifyHint {
            binding: EvmVerifyBinding::EffectContractFromReceipt,
            post_evm_request: None,
        }),
    );

    ActionGraphFragment {
        roots: vec![simulate_id],
        terminals: vec![verify_id.clone()],
        nodes,
        live_binding_hints,
    }
}

fn direct_generic_fragment(
    provider_id: &str,
    chain: &str,
    envelope: RuntimeEnvelope,
    effect_contract: &EffectContract,
) -> ActionGraphFragment {
    let actuate_id = format!("api_native.{provider_id}.actuate");
    let verify_id = format!("api_native.{provider_id}.verify");
    let mode = if envelope.kind == RuntimeEnvelopeKind::ExternalJob {
        ActuateMode::ExternalJob
    } else {
        ActuateMode::ApiNativeEnvelope
    };

    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        actuate_id.clone(),
        ActionNode {
            node_id: actuate_id.clone(),
            kind: ActionNodeKind::Actuate,
            origin: ActionOrigin::ApiNativePath,
            status: ActionNodeStatus::Pending,
            depends_on: Vec::new(),
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Actuate(ActuateAction {
                mode,
                actuator_hint: format!("api-native {provider_id} direct envelope"),
                chain: Some(chain.to_owned()),
                envelope_ref: Some(envelope.envelope_id.clone()),
                requires_effect_contract: true,
                live: None,
            }),
            implementation_hint: Some(provider_id.to_owned()),
            expected_effect_ref: Some(effect_contract.effect_id.clone()),
        },
    );
    nodes.insert(
        verify_id.clone(),
        ActionNode {
            node_id: verify_id.clone(),
            kind: ActionNodeKind::Verify,
            origin: ActionOrigin::ApiNativePath,
            status: ActionNodeStatus::Pending,
            depends_on: vec![actuate_id.clone()],
            inputs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: ActionPayload::Verify(VerifyAction {
                verify_kind: VerifyKind::EffectContract,
                verifier_hint: format!("api-native {provider_id} effect verify"),
                pre_observation_ref: None,
                post_observation_ref: None,
                live: None,
            }),
            implementation_hint: Some(provider_id.to_owned()),
            expected_effect_ref: Some(effect_contract.effect_id.clone()),
        },
    );

    ActionGraphFragment {
        roots: vec![actuate_id],
        terminals: vec![verify_id],
        nodes,
        live_binding_hints: std::collections::BTreeMap::new(),
    }
}
