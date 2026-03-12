use serde_json::json;

use crate::{
    envelope::{
        bind_raw_envelope_action, ensure_raw_envelope_broadcastable, RawEnvelopeGateError,
        RuntimeEnvelope, RuntimeEnvelopeKind,
    },
    governor::GovernorDecision,
};

#[test]
fn binding_raw_envelope_requires_effect_contract_ref() {
    let envelope = sample_envelope();

    let error = bind_raw_envelope_action("swap", &envelope, None, "broadcast swap")
        .expect_err("missing effect contract should fail");

    assert_eq!(error, RawEnvelopeGateError::MissingEffectContract);
}

#[test]
fn raw_envelope_broadcast_gate_requires_governor_allowance() {
    let envelope = sample_envelope();
    let action = bind_raw_envelope_action(
        "swap",
        &envelope,
        Some("effects.swap".to_owned()),
        "broadcast swap",
    )
    .expect("bound action");

    assert!(ensure_raw_envelope_broadcastable(&action, &GovernorDecision::Allow).is_ok());
    assert_eq!(
        ensure_raw_envelope_broadcastable(&action, &GovernorDecision::Reject),
        Err(RawEnvelopeGateError::GovernorRejected)
    );
}

fn sample_envelope() -> RuntimeEnvelope {
    RuntimeEnvelope {
        envelope_id: "env-1".to_owned(),
        kind: RuntimeEnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"to":"0xabc","data":"0xdeadbeef","value":"0"}),
        provenance: Some("host.quote_api".to_owned()),
    }
}
