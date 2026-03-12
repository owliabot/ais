use crate::{
    action::kinds::actuate::{ActuateAction, ActuateMode},
    action::{ActionNode, ActionNodeKind, ActionNodeStatus, ActionOrigin, ActionPayload},
    governor::GovernorDecision,
};

use super::RuntimeEnvelope;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RawEnvelopeGateError {
    #[error("raw envelope action requires an effect contract reference")]
    MissingEffectContract,
    #[error("raw envelope action requires an envelope reference")]
    MissingEnvelopeRef,
    #[error("governor decision does not permit raw envelope broadcast")]
    GovernorRejected,
}

pub fn bind_raw_envelope_action(
    node_id: impl Into<String>,
    envelope: &RuntimeEnvelope,
    effect_contract_ref: Option<String>,
    actuator_hint: impl Into<String>,
) -> Result<ActionNode, RawEnvelopeGateError> {
    let effect_contract_ref =
        effect_contract_ref.ok_or(RawEnvelopeGateError::MissingEffectContract)?;
    Ok(ActionNode {
        node_id: node_id.into(),
        kind: ActionNodeKind::Actuate,
        origin: ActionOrigin::RawEnvelopePath,
        status: ActionNodeStatus::Pending,
        depends_on: Vec::new(),
        inputs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: ActionPayload::Actuate(ActuateAction {
            mode: ActuateMode::RawEnvelope,
            actuator_hint: actuator_hint.into(),
            chain: Some(envelope.chain.clone()),
            envelope_ref: Some(envelope.envelope_id.clone()),
            requires_effect_contract: true,
            live: None,
        }),
        implementation_hint: envelope.provenance.clone(),
        expected_effect_ref: Some(effect_contract_ref),
    })
}

pub fn ensure_raw_envelope_broadcastable(
    action: &ActionNode,
    governor_decision: &GovernorDecision,
) -> Result<(), RawEnvelopeGateError> {
    let ActionPayload::Actuate(actuate) = &action.payload else {
        return Err(RawEnvelopeGateError::MissingEnvelopeRef);
    };

    if actuate.mode != ActuateMode::RawEnvelope {
        return Ok(());
    }

    if actuate.envelope_ref.is_none() {
        return Err(RawEnvelopeGateError::MissingEnvelopeRef);
    }

    if action.expected_effect_ref.is_none() || !actuate.requires_effect_contract {
        return Err(RawEnvelopeGateError::MissingEffectContract);
    }

    match governor_decision {
        GovernorDecision::Allow | GovernorDecision::AllowWithSigner => Ok(()),
        GovernorDecision::RequireMoreEvidence | GovernorDecision::Reject => {
            Err(RawEnvelopeGateError::GovernorRejected)
        }
    }
}
