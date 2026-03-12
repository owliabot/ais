use std::collections::BTreeMap;

use ais_agent_chain_shared::ChainFamily;
use ais_agent_core::{
    action::{
        kinds::actuate::{ActuateLiveBinding, ActuateMode, EvmActuateLiveBinding},
        ActionOrigin, ActionPayload,
    },
    binding::evm::EvmActuateBinding,
    evidence::EvidenceGraph,
    mission::{Mission, MissionBudget, MissionPolicy},
};
use serde_json::json;

use crate::reflect::{
    EvmAbiReflectionAdapter, ReflectionArtifactKind, ReflectionDriver, ReflectionRequest,
    SolanaIdlReflectionAdapter,
};

#[test]
fn evm_reflection_adapter_emits_standard_fragment_shape() {
    let adapter = EvmAbiReflectionAdapter;
    let output = adapter
        .build(&ReflectionRequest {
            mission: sample_mission("eip155:1"),
            evidence: EvidenceGraph::default(),
            chain_family: ChainFamily::Evm,
            artifact_kind: ReflectionArtifactKind::EvmAbi,
            artifact: json!({"name":"Router","methods":["swapExactIn"]}),
            action_selector: "swapExactIn".to_owned(),
        })
        .expect("evm reflection");

    assert!(output
        .fragment
        .live_binding_hints
        .contains_key("reflect.evm.swapExactIn"));

    let mut output = output;
    output
        .apply_live_binding_hints()
        .expect("evm reflection live binding hints should apply");

    let node = output
        .fragment
        .nodes
        .get("reflect.evm.swapExactIn")
        .expect("reflected node");
    assert_eq!(node.origin, ActionOrigin::ReflectionPath);
    match &node.payload {
        ActionPayload::Actuate(action) => {
            assert_eq!(action.mode, ActuateMode::ReflectedCall);
            assert!(action.requires_effect_contract);
            assert_eq!(
                action.live,
                Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                    connection: None,
                    binding: EvmActuateBinding::BroadcastTypedTransaction,
                }))
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }
    assert_eq!(output.effect_contracts.len(), 1);
}

#[test]
fn solana_reflection_adapter_emits_standard_fragment_shape() {
    let adapter = SolanaIdlReflectionAdapter;
    let output = adapter
        .build(&ReflectionRequest {
            mission: sample_mission("solana:mainnet"),
            evidence: EvidenceGraph::default(),
            chain_family: ChainFamily::Solana,
            artifact_kind: ReflectionArtifactKind::SolanaIdl,
            artifact: json!({"name":"TokenProgram","instructions":["transfer"]}),
            action_selector: "transfer".to_owned(),
        })
        .expect("solana reflection");

    let node = output
        .fragment
        .nodes
        .get("reflect.solana.transfer")
        .expect("reflected node");
    assert_eq!(node.origin, ActionOrigin::ReflectionPath);
    assert_eq!(output.effect_contracts.len(), 1);
}

fn sample_mission(chain: &str) -> Mission {
    Mission {
        mission_id: "mission-1".to_owned(),
        goal: "execute reflected action".to_owned(),
        allowed_chains: vec![chain.to_owned()],
        budget: MissionBudget::default(),
        policy: MissionPolicy::default(),
        constraints: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}
