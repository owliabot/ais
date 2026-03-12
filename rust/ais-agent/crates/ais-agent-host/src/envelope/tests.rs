use serde_json::json;

use ais_agent_control::ids::RunId;

use crate::envelope::{HostEnvelopeKind, HostEnvelopeSubmission};

#[test]
fn host_envelope_submission_converts_to_runtime_envelope() {
    let runtime = HostEnvelopeSubmission {
        run_id: RunId("run-1".to_owned()),
        envelope_id: "env-1".to_owned(),
        kind: HostEnvelopeKind::EvmEnvelope,
        chain: "eip155:1".to_owned(),
        payload: json!({"to":"0xabc"}),
        expected_effect_ref: Some("effects.swap".to_owned()),
        expected_effect_contract: None,
        provenance: Some("host.quote_api".to_owned()),
    }
    .into_runtime_envelope();

    assert_eq!(runtime.envelope_id, "env-1");
    assert_eq!(runtime.chain, "eip155:1");
    assert_eq!(runtime.provenance.as_deref(), Some("host.quote_api"));
}
