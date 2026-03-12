use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    action::{
        kinds::{
            actuate::{ActuateLiveBinding, EvmActuateLiveBinding},
            observe::{EvmObserveLiveBinding, ObserveLiveBinding},
            simulate::{EvmSimulateLiveBinding, SimulateLiveBinding},
            verify::{EvmVerifyLiveBinding, VerifyLiveBinding},
        },
        ActionNode, ActionNodeKind, ActionPayload,
    },
    effect::EffectContract,
    evidence::EvidenceRequirement,
};

use super::{DriverFragmentBindingError, DriverNodeLiveBindingHint};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionGraphFragment {
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub terminals: Vec<String>,
    #[serde(default)]
    pub nodes: BTreeMap<String, ActionNode>,
    #[serde(default)]
    pub live_binding_hints: BTreeMap<String, DriverNodeLiveBindingHint>,
}

impl ActionGraphFragment {
    pub fn apply_live_binding_hints(&mut self) -> Result<(), DriverFragmentBindingError> {
        for (node_id, hint) in self.live_binding_hints.clone() {
            let node = self.nodes.get_mut(&node_id).ok_or_else(|| {
                DriverFragmentBindingError::NodeNotFound {
                    node_id: node_id.clone(),
                }
            })?;

            apply_hint_to_node(node, hint)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DriverBuildOutput {
    pub fragment: ActionGraphFragment,
    #[serde(default)]
    pub evidence_requirements: Vec<EvidenceRequirement>,
    #[serde(default)]
    pub effect_contracts: Vec<EffectContract>,
}

impl DriverBuildOutput {
    pub fn apply_live_binding_hints(&mut self) -> Result<(), DriverFragmentBindingError> {
        self.fragment.apply_live_binding_hints()
    }
}

fn apply_hint_to_node(
    node: &mut ActionNode,
    hint: DriverNodeLiveBindingHint,
) -> Result<(), DriverFragmentBindingError> {
    match (node.kind.clone(), &mut node.payload, hint) {
        (
            ActionNodeKind::Observe,
            ActionPayload::Observe(action),
            DriverNodeLiveBindingHint::EvmObserve(hint),
        ) => {
            let connection = match action.live.take() {
                Some(ObserveLiveBinding::Evm(live)) => live.connection,
                _ => None,
            };
            action.live = Some(ObserveLiveBinding::Evm(EvmObserveLiveBinding {
                connection,
                binding: hint.binding,
                request: hint.request,
            }));
            Ok(())
        }
        (
            ActionNodeKind::Simulate,
            ActionPayload::Simulate(action),
            DriverNodeLiveBindingHint::EvmSimulate(hint),
        ) => {
            let connection = match action.live.take() {
                Some(SimulateLiveBinding::Evm(live)) => live.connection,
                _ => None,
            };
            action.live = Some(SimulateLiveBinding::Evm(EvmSimulateLiveBinding {
                connection,
                binding: hint.binding,
                request: hint.request,
            }));
            Ok(())
        }
        (
            ActionNodeKind::Actuate,
            ActionPayload::Actuate(action),
            DriverNodeLiveBindingHint::EvmActuate(hint),
        ) => {
            let connection = match action.live.take() {
                Some(ActuateLiveBinding::Evm(live)) => live.connection,
                _ => None,
            };
            action.live = Some(ActuateLiveBinding::Evm(EvmActuateLiveBinding {
                connection,
                binding: hint.binding,
            }));
            Ok(())
        }
        (
            ActionNodeKind::Verify,
            ActionPayload::Verify(action),
            DriverNodeLiveBindingHint::EvmVerify(hint),
        ) => {
            let (connection, existing_post_request) = match action.live.take() {
                Some(VerifyLiveBinding::Evm(live)) => (live.connection, live.post_request),
                _ => (None, None),
            };
            action.live = Some(VerifyLiveBinding::Evm(EvmVerifyLiveBinding {
                connection,
                binding: hint.binding,
                post_request: hint.post_evm_request.or(existing_post_request),
            }));
            Ok(())
        }
        (kind, _, hint) => Err(DriverFragmentBindingError::KindMismatch {
            node_id: node.node_id.clone(),
            node_kind: kind,
            hint_kind: hint.kind_name().to_owned(),
        }),
    }
}
