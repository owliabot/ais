use serde_json::json;

use ais_agent_control::ids::{RunId, SignerRequestId};
use ais_agent_core::evidence::EvidenceKind;

use crate::{
    envelope::{HostEnvelopeKind, HostEnvelopeSubmission},
    evidence::HostEvidenceSubmission,
    ingest::{HostIngestKind, HostIngestSubmission},
    signer::{HostSignerDecision, HostSignerDecisionKind},
};

#[test]
fn unified_ingest_surface_reports_kind_and_run_id() {
    let evidence = HostIngestSubmission::Evidence(HostEvidenceSubmission {
        run_id: RunId("run-a".to_owned()),
        evidence_id: "evidence-1".to_owned(),
        kind: EvidenceKind::Fact,
        source: "host".to_owned(),
        observed_at_ms: None,
        expires_at_ms: None,
        max_age_ms: None,
        chain_scope: None,
        trace_hint: None,
        confidence_ppm: None,
        payload: json!({"ok":true}),
    });
    assert_eq!(evidence.kind(), HostIngestKind::Evidence);
    assert_eq!(evidence.run_id().0, "run-a");

    let envelope = HostIngestSubmission::Envelope(HostEnvelopeSubmission {
        run_id: RunId("run-b".to_owned()),
        envelope_id: "env-1".to_owned(),
        kind: HostEnvelopeKind::ExternalJob,
        chain: "offchain".to_owned(),
        payload: json!({"job":"x"}),
        expected_effect_ref: None,
        expected_effect_contract: None,
        provenance: None,
    });
    assert_eq!(envelope.kind(), HostIngestKind::Envelope);
    assert_eq!(envelope.run_id().0, "run-b");

    let signer = HostIngestSubmission::SignerDecision(HostSignerDecision {
        run_id: RunId("run-c".to_owned()),
        request_id: SignerRequestId("signer-1".to_owned()),
        decision: HostSignerDecisionKind::Approved,
        decided_at_ms: None,
        tx_hash: None,
        details: Default::default(),
    });
    assert_eq!(signer.kind(), HostIngestKind::SignerDecision);
    assert_eq!(signer.run_id().0, "run-c");
}
