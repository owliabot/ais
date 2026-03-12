use serde_json::json;

use ais_agent_control::ids::RunId;
use ais_agent_core::evidence::EvidenceKind;

use crate::evidence::HostEvidenceSubmission;

#[test]
fn host_evidence_submission_converts_to_runtime_evidence_record() {
    let record = HostEvidenceSubmission {
        run_id: RunId("run-1".to_owned()),
        evidence_id: "evidence.quote.1".to_owned(),
        kind: EvidenceKind::RouteOrQuote,
        source: "host.quote_api".to_owned(),
        observed_at_ms: Some(100),
        expires_at_ms: Some(200),
        max_age_ms: Some(50),
        chain_scope: Some("eip155:1".to_owned()),
        trace_hint: Some("trace-1".to_owned()),
        confidence_ppm: Some(950_000),
        payload: json!({"amount_out":"1000"}),
    }
    .into_evidence_record();

    assert_eq!(record.evidence_id, "evidence.quote.1");
    assert_eq!(record.provenance.source, "host.quote_api");
    assert_eq!(record.provenance.chain_scope.as_deref(), Some("eip155:1"));
    assert_eq!(record.freshness.observed_at_ms, Some(100));
    assert_eq!(record.confidence_ppm, Some(950_000));
}
