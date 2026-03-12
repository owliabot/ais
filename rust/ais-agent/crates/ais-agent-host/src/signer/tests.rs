use ais_agent_control::ids::{RunId, SignerRequestId};

use crate::signer::{HostSignerDecision, HostSignerDecisionKind};

#[test]
fn host_signer_decision_converts_to_runtime_decision() {
    let runtime = HostSignerDecision {
        run_id: RunId("run-1".to_owned()),
        request_id: SignerRequestId("signer-1".to_owned()),
        decision: HostSignerDecisionKind::Submitted,
        decided_at_ms: Some(100),
        tx_hash: Some("0xabc".to_owned()),
        details: Default::default(),
    }
    .into_runtime_decision();

    assert_eq!(runtime.request_id.0, "signer-1");
    assert_eq!(runtime.tx_hash.as_deref(), Some("0xabc"));
}
