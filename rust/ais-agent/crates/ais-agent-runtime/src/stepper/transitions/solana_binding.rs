//! Typed Solana live-binding resolution helpers.

use ais_agent_core::{
    action::{
        kinds::{
            actuate::ActuateLiveBinding, observe::ObserveLiveBinding,
            simulate::SimulateLiveBinding, verify::VerifyLiveBinding,
        },
        ActionNode, ActionNodeKind, ActionPayload,
    },
    binding::solana::{
        SolanaActuateBinding, SolanaObserveBinding, SolanaSimulateBinding, SolanaVerifyBinding,
    },
};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_solana_observe_binding(node: &ActionNode) -> Option<SolanaObserveBinding> {
    if node.kind != ActionNodeKind::Observe {
        return None;
    }

    match &node.payload {
        ActionPayload::Observe(observe) => match &observe.live {
            Some(ObserveLiveBinding::Solana(live)) => Some(live.binding.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_solana_simulate_binding(node: &ActionNode) -> Option<SolanaSimulateBinding> {
    if node.kind != ActionNodeKind::Simulate {
        return None;
    }

    match &node.payload {
        ActionPayload::Simulate(simulate) => match &simulate.live {
            Some(SimulateLiveBinding::Solana(live)) => Some(live.binding.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_solana_actuate_binding(node: &ActionNode) -> Option<SolanaActuateBinding> {
    if node.kind != ActionNodeKind::Actuate {
        return None;
    }

    match &node.payload {
        ActionPayload::Actuate(actuate) => match &actuate.live {
            Some(ActuateLiveBinding::Solana(live)) => Some(live.binding.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_solana_verify_binding(node: &ActionNode) -> Option<SolanaVerifyBinding> {
    if node.kind != ActionNodeKind::Verify {
        return None;
    }

    match &node.payload {
        ActionPayload::Verify(verify) => match &verify.live {
            Some(VerifyLiveBinding::Solana(live)) => Some(live.binding.clone()),
            _ => None,
        },
        _ => None,
    }
}
