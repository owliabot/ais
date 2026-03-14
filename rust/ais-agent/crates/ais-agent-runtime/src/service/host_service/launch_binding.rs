use ais_agent_control::launch_spec::PrebuiltFragmentLaunchSpec;
use ais_agent_core::{
    action::ActionGraph, checkpoint::CheckpointSnapshot, effect::EffectContract,
    evidence::EvidenceGraph,
};

use super::launch_validation::ValidatedPrebuiltFragment;

pub(crate) fn seed_prebuilt_fragment_checkpoint(
    checkpoint: &mut CheckpointSnapshot,
    validated: ValidatedPrebuiltFragment,
) {
    if let Some(action_graph) = validated.action_graph {
        checkpoint.action_graph = action_graph;
    }

    if let Some(evidence_graph) = validated.evidence_graph {
        checkpoint.evidence_graph = evidence_graph;
    }

    if let Some(effect_contracts) = validated.effect_contracts {
        checkpoint.effect_contracts = effect_contracts;
    }
}

pub(crate) fn parse_prebuilt_fragment(
    spec: &PrebuiltFragmentLaunchSpec,
) -> Result<ValidatedPrebuiltFragment, String> {
    Ok(ValidatedPrebuiltFragment {
        action_graph: spec
            .action_graph
            .as_ref()
            .map(parse_action_graph)
            .transpose()?,
        evidence_graph: spec
            .evidence_graph
            .as_ref()
            .map(parse_evidence_graph)
            .transpose()?,
        effect_contracts: spec
            .effect_contracts
            .as_ref()
            .map(parse_effect_contracts)
            .transpose()?,
    })
}

fn parse_action_graph(value: &serde_json::Value) -> Result<ActionGraph, String> {
    serde_json::from_value::<ActionGraph>(value.clone())
        .map_err(|error| format!("invalid prebuilt_fragment.action_graph: {error}"))
}

fn parse_evidence_graph(value: &serde_json::Value) -> Result<EvidenceGraph, String> {
    serde_json::from_value::<EvidenceGraph>(value.clone())
        .map_err(|error| format!("invalid prebuilt_fragment.evidence_graph: {error}"))
}

fn parse_effect_contracts(
    value: &serde_json::Value,
) -> Result<std::collections::BTreeMap<String, EffectContract>, String> {
    serde_json::from_value::<std::collections::BTreeMap<String, EffectContract>>(value.clone())
        .map_err(|error| format!("invalid prebuilt_fragment.effect_contracts: {error}"))
}
