use std::collections::BTreeMap;

use ais_agent_chain_shared::{
    ChainFamily, ReflectionArtifactKind, ReflectionDriver, ReflectionDriverError,
    ReflectionDriverOutput, ReflectionRequest,
};
use ais_agent_core::{
    action::{
        kinds::actuate::{ActuateAction, ActuateMode},
        ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload,
    },
    driver::{ActionGraphFragment, DriverBuildOutput},
    effect::{EffectAssertion, EffectContract, EffectContractKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolanaIdlReflectionAdapter;

impl ReflectionDriver for SolanaIdlReflectionAdapter {
    fn driver_id(&self) -> &'static str {
        "reflect.solana_idl"
    }

    fn family(&self) -> ChainFamily {
        ChainFamily::Solana
    }

    fn build(
        &self,
        request: &ReflectionRequest,
    ) -> Result<ReflectionDriverOutput, ReflectionDriverError> {
        if request.chain_family != ChainFamily::Solana {
            return Err(ReflectionDriverError::UnsupportedFamily(
                request.chain_family.clone(),
            ));
        }
        if request.artifact_kind != ReflectionArtifactKind::SolanaIdl {
            return Err(ReflectionDriverError::UnsupportedArtifact);
        }

        let chain = request
            .mission
            .allowed_chains
            .first()
            .cloned()
            .unwrap_or_else(|| "solana:unknown".to_owned());
        let node_id = format!("reflect.solana.{}", request.action_selector);

        let mut nodes = BTreeMap::new();
        nodes.insert(
            node_id.clone(),
            ActionNode {
                node_id: node_id.clone(),
                kind: ActionNodeKind::Actuate,
                origin: ActionOrigin::ReflectionPath,
                status: ActionNodeStatus::Pending,
                depends_on: Vec::new(),
                inputs: Vec::new(),
                evidence_refs: Vec::new(),
                payload: ActionPayload::Actuate(ActuateAction {
                    mode: ActuateMode::ReflectedCall,
                    actuator_hint: format!(
                        "reflect solana idl instruction {}",
                        request.action_selector
                    ),
                    chain: Some(chain.clone()),
                    envelope_ref: None,
                    requires_effect_contract: true,
                    live: None,
                }),
                implementation_hint: Some(self.driver_id().to_owned()),
                expected_effect_ref: Some(format!("effects.{node_id}")),
            },
        );

        Ok(DriverBuildOutput {
            fragment: ActionGraphFragment {
                roots: vec![node_id.clone()],
                terminals: vec![node_id.clone()],
                nodes,
                live_binding_hints: BTreeMap::new(),
            },
            evidence_requirements: Vec::new(),
            effect_contracts: vec![EffectContract {
                effect_id: format!("effects.{node_id}"),
                kind: EffectContractKind::StateTransition,
                assertions: vec![EffectAssertion {
                    expression: "receipt != null".to_owned(),
                    description: "reflected Solana instruction should yield a receipt".to_owned(),
                }],
                tolerance_hint: Some("receipt_required".to_owned()),
            }],
        })
    }
}
